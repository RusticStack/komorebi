//! Tables: the auto layout algorithm with colspan and rowspan, cell
//! spacing, collapsed borders and row/row-group backgrounds. Children of a
//! row that are not cells (a `td { display: block }` media rule) become one
//! anonymous cell whose content stacks as blocks, as CSS specifies.

use super::dom::Id;
use super::layout::{Item, Layouter, Rect, fin, shift};
use super::style::{Color, Display, Len, Radius, Style, VAlign};

/// Cells laid out per message; later cells are dropped.
pub const MAX_CELLS: usize = 20_000;
/// `colspan` and `rowspan` are clamped to this.
pub const MAX_SPAN: usize = 1_000;
/// Columns per table.
const MAX_COLS: usize = 1_000;
/// Grid slots (colspan x rowspan) per table before spans are ignored.
const MAX_SLOTS: usize = 200_000;

pub(super) struct CellInfo {
    node: Id,
    /// Anonymous cell: these children of the row (`node`) are its content.
    anon: Vec<Id>,
    row: usize,
    col: usize,
    cs: usize,
    rs: usize,
    min: f32,
    max: f32,
    spec: Len,
    hb: f32,
    vb: f32,
    /// What the cell's edges take: its border, or with collapsed borders its
    /// half of the shared line. Top, right, bottom, left.
    ins: [f32; 4],
}

/// One segment of a collapsed border.
#[derive(Clone, Copy, Default)]
struct Line {
    w: f32,
    color: Color,
    /// Who set it: table, row group, row, cell. On equal widths the higher
    /// rank wins, and among cells the one above or to the left.
    rank: u8,
}

impl Line {
    fn offer(&mut self, w: f32, color: Color, rank: u8) {
        if w > 0.0 && (w > self.w || (w == self.w && rank > self.rank)) {
            *self = Line { w, color, rank };
        }
    }
}

/// The grid of a table's collapsed borders.
struct Collapsed {
    /// Above row `r` at column `c`: `r * ncols + c`, `nrows + 1` rows of them.
    above: Vec<Line>,
    /// Left of column `c` in row `r`: `r * (ncols + 1) + c`.
    left: Vec<Line>,
    /// The half of the outer lines the table adds to its columns. Top, right,
    /// bottom, left.
    edge: [f32; 4],
}

pub(super) struct TableInfo {
    cells: Vec<CellInfo>,
    rows: Vec<Id>,
    /// The row group of each row.
    groups: Vec<Option<Id>>,
    ncols: usize,
    col_min: Vec<f32>,
    col_max: Vec<f32>,
    col_spec: Vec<Len>,
    /// With `border-collapse: collapse`, unless the grid is absurdly large.
    collapsed: Option<Collapsed>,
}

fn span_attr(v: Option<&str>) -> usize {
    v.and_then(|v| v.trim().parse::<usize>().ok())
        .unwrap_or(1)
        .clamp(1, MAX_SPAN)
}

impl Layouter<'_> {
    /// The table's grid and column widths, built once; the caller puts it
    /// back into `self.tables`.
    fn table_info(&mut self, id: Id) -> TableInfo {
        match self.tables.remove(&id) {
            Some(t) => t,
            None => self.build_table(id),
        }
    }

    fn build_table(&mut self, id: Id) -> TableInfo {
        let mut rows: Vec<Id> = vec![];
        let mut groups: Vec<Option<Id>> = vec![];
        for &c in self.dom.children(id) {
            match self.style(c).display {
                Display::Row => {
                    rows.push(c);
                    groups.push(None);
                }
                Display::RowGroup => {
                    for &r in self.dom.children(c) {
                        if self.style(r).display == Display::Row {
                            rows.push(r);
                            groups.push(Some(c));
                        }
                    }
                }
                _ => {}
            }
        }
        let nrows = rows.len();
        let mut occ: Vec<Vec<bool>> = vec![vec![]; nrows];
        let mut cells: Vec<CellInfo> = vec![];
        let mut slots = 0usize;
        for (ri, &r) in rows.iter().enumerate() {
            if self.expired() || self.cells >= MAX_CELLS {
                break;
            }
            // Cells, with runs of non-cell children grouped into one.
            let mut units: Vec<(Id, Vec<Id>)> = vec![];
            for &c in self.dom.children(r) {
                if !self.dom.is_elem(c) {
                    continue;
                }
                match self.style(c).display {
                    Display::Cell => units.push((c, vec![])),
                    Display::None => {}
                    _ => match units.last_mut() {
                        Some((_, anon)) if !anon.is_empty() => anon.push(c),
                        _ => units.push((r, vec![c])),
                    },
                }
            }
            let mut col = 0usize;
            for (node, anon) in units {
                if self.cells >= MAX_CELLS {
                    break;
                }
                let row_occ = occ.get(ri).map_or(&[][..], |o| o.as_slice());
                while row_occ.get(col).copied().unwrap_or(false) {
                    col = col.saturating_add(1);
                }
                if col >= MAX_COLS {
                    break;
                }
                self.cells = self.cells.saturating_add(1);
                let (mut cs, mut rs) = if anon.is_empty() {
                    (
                        span_attr(self.dom.attr(node, "colspan")),
                        span_attr(self.dom.attr(node, "rowspan")),
                    )
                } else {
                    (1, 1)
                };
                cs = cs.min(MAX_COLS.saturating_sub(col)).max(1);
                rs = rs.min(nrows.saturating_sub(ri)).max(1);
                slots = slots.saturating_add(cs.saturating_mul(rs));
                if slots > MAX_SLOTS {
                    (cs, rs) = (1, 1);
                }
                let end = col.saturating_add(cs);
                for o in occ.iter_mut().skip(ri).take(rs) {
                    if o.len() < end {
                        o.resize(end, false);
                    }
                    for x in o.iter_mut().skip(col).take(cs) {
                        *x = true;
                    }
                }
                let (cell, mn, mx) = if anon.is_empty() {
                    let s = self.style(node).clone();
                    let (mn, mx) = self.content_widths(node);
                    // A collapsed border is added once the lines are known.
                    let own = if self.style(id).collapse {
                        [0.0; 4]
                    } else {
                        s.border
                    };
                    let hb = s.padding[1] + s.padding[3] + own[1] + own[3];
                    let vb = s.padding[0] + s.padding[2] + own[0] + own[2];
                    let (mn, mx) = match s.width {
                        Len::Px(w) => {
                            let w = if s.border_box { fin(w - hb) } else { w };
                            let w = w.min(self.maxw);
                            (mn.min(w.max(mn)), w.max(mn))
                        }
                        _ => (mn, mx),
                    };
                    // A width is the content box's unless `box-sizing` says
                    // otherwise; the column holds the padding and border too.
                    let spec = match s.width {
                        Len::Px(w) if !s.border_box => Len::Px(w + hb),
                        w => w,
                    };
                    ((spec, hb, vb, own), mn, mx)
                } else {
                    let (mn, mx) = self.list_widths(node, &anon);
                    ((Len::Auto, 0.0, 0.0, [0.0; 4]), mn, mx)
                };
                let (spec, hb, vb, ins) = cell;
                cells.push(CellInfo {
                    node,
                    anon,
                    row: ri,
                    col,
                    cs,
                    rs,
                    min: mn + hb,
                    max: mx + hb,
                    spec,
                    hb,
                    vb,
                    ins,
                });
                col = end;
            }
        }
        let ncols = occ.iter().map(Vec::len).max().unwrap_or(0);
        let collapsed = (self.style(id).collapse && nrows.saturating_mul(ncols) <= MAX_SLOTS)
            .then(|| self.collapse_borders(id, &rows, &groups, &mut cells, ncols));
        let mut col_min = vec![0.0f32; ncols];
        let mut col_max = vec![0.0f32; ncols];
        let mut col_spec = vec![Len::Auto; ncols];
        for c in cells.iter().filter(|c| c.cs == 1) {
            let (Some(mn), Some(mx), Some(sp)) = (
                col_min.get_mut(c.col),
                col_max.get_mut(c.col),
                col_spec.get_mut(c.col),
            ) else {
                continue;
            };
            *mn = mn.max(c.min);
            *mx = mx.max(c.max).max(*mn);
            *sp = match (c.spec, *sp) {
                (Len::Px(_) | Len::Pct(_), Len::Auto) => c.spec,
                (Len::Px(a), Len::Px(b)) => Len::Px(a.max(b)),
                (_, old) => old,
            };
        }
        for c in cells.iter().filter(|c| c.cs > 1) {
            let span = c.col..c.col.saturating_add(c.cs).min(ncols);
            let n = span.len().max(1) as f32;
            for (cols, want) in [(&mut col_min, c.min), (&mut col_max, c.max)] {
                let Some(cols) = cols.get_mut(span.clone()) else {
                    continue;
                };
                let have: f32 = cols.iter().sum();
                if want > have {
                    let add = (want - have) / n;
                    cols.iter_mut().for_each(|w| *w += add);
                }
            }
        }
        for (k, w) in self.col_widths(id, ncols).into_iter().enumerate() {
            if w == Len::Auto {
                continue;
            }
            if let Some(sp) = col_spec.get_mut(k) {
                *sp = match (w, *sp) {
                    (Len::Px(a), Len::Px(b)) => Len::Px(a.max(b)),
                    (w, _) => w,
                };
            }
            if let (Len::Px(a), Some(mx)) = (w, col_max.get_mut(k)) {
                *mx = mx.max(a);
            }
        }
        for (mx, mn) in col_max.iter_mut().zip(&col_min) {
            *mx = mx.max(*mn);
        }
        TableInfo {
            cells,
            rows,
            groups,
            ncols,
            col_min,
            col_max,
            col_spec,
            collapsed,
        }
    }

    /// The widths `<col>` elements (or a `<colgroup>` without any) declare,
    /// by column; `Auto` for the rest.
    fn col_widths(&self, id: Id, ncols: usize) -> Vec<Len> {
        let mut out = vec![];
        let span = |c: Id| span_attr(self.dom.attr(c, "span"));
        let mut add = |w: Len, n: usize| {
            let room = ncols.saturating_sub(out.len());
            out.extend(std::iter::repeat_n(w, n.min(room)));
        };
        for &g in self.dom.children(id) {
            match self.dom.tag(g) {
                "col" => add(self.style(g).width, span(g)),
                "colgroup" => {
                    let mut any = false;
                    for &c in self.dom.children(g) {
                        if self.dom.tag(c) == "col" {
                            any = true;
                            let w = self.style(c).width;
                            add(
                                if w == Len::Auto {
                                    self.style(g).width
                                } else {
                                    w
                                },
                                span(c),
                            );
                        }
                    }
                    if !any {
                        add(self.style(g).width, span(g));
                    }
                }
                _ => {}
            }
        }
        out
    }

    /// The column widths of a `table-layout: fixed` table: those declared by
    /// `<col>` and the first row, the rest sharing what is left. Content does
    /// not count; it overflows.
    fn fixed_columns(&self, id: Id, ti: &TableInfo, avail: f32) -> Vec<f32> {
        let ncols = ti.ncols;
        let mut spec = self.col_widths(id, ncols);
        spec.resize(ncols, Len::Auto);
        for c in ti.cells.iter().filter(|c| c.row == 0) {
            let n = c.cs.max(1) as f32;
            for k in c.col..c.col.saturating_add(c.cs).min(ncols) {
                if let Some(sp @ Len::Auto) = spec.get_mut(k) {
                    *sp = match c.spec {
                        Len::Px(w) => Len::Px(w / n),
                        Len::Pct(p) => Len::Pct(p / n),
                        Len::Auto => Len::Auto,
                    };
                }
            }
        }
        let mut colw: Vec<f32> = spec
            .iter()
            .map(|l| match *l {
                Len::Px(p) => p,
                Len::Pct(p) => avail * p / 100.0,
                Len::Auto => 0.0,
            })
            .collect();
        let taken: f32 = colw.iter().sum();
        let autos = spec.iter().filter(|l| **l == Len::Auto).count();
        let left = fin(avail - taken);
        if autos > 0 {
            let share = left / autos as f32;
            for (w, l) in colw.iter_mut().zip(&spec) {
                if *l == Len::Auto {
                    *w = share;
                }
            }
        } else if taken > 0.0 {
            // All declared: they keep their proportions in the table's width.
            let k = avail / taken;
            colw.iter_mut().for_each(|w| *w *= k);
        }
        colw
    }

    /// Resolve the collapsed borders of a table (CSS 2 17.6.2.1: the wider
    /// wins, then cell over row over row group over table) and give each cell
    /// its half of the lines around it.
    fn collapse_borders(
        &self,
        id: Id,
        rows: &[Id],
        groups: &[Option<Id>],
        cells: &mut [CellInfo],
        ncols: usize,
    ) -> Collapsed {
        let nrows = rows.len();
        let (nabove, nleft) = (
            nrows.saturating_add(1).saturating_mul(ncols),
            nrows.saturating_mul(ncols.saturating_add(1)),
        );
        let mut above = vec![Line::default(); nabove];
        let mut left = vec![Line::default(); nleft];
        let at_above = |r: usize, c: usize| r.saturating_mul(ncols).saturating_add(c);
        let at_left =
            |r: usize, c: usize| r.saturating_mul(ncols.saturating_add(1)).saturating_add(c);
        // The box from row r0 to r1 and column c0 to c1 (excluded) offers its
        // border to the lines around it.
        let mut offer = |s: &Style, r0: usize, r1: usize, c0: usize, c1: usize, rank: u8| {
            for c in c0..c1 {
                if let Some(l) = above.get_mut(at_above(r0, c)) {
                    l.offer(s.border[0], s.border_color[0], rank);
                }
                if let Some(l) = above.get_mut(at_above(r1, c)) {
                    l.offer(s.border[2], s.border_color[2], rank);
                }
            }
            for r in r0..r1 {
                if let Some(l) = left.get_mut(at_left(r, c0)) {
                    l.offer(s.border[3], s.border_color[3], rank);
                }
                if let Some(l) = left.get_mut(at_left(r, c1)) {
                    l.offer(s.border[1], s.border_color[1], rank);
                }
            }
        };
        offer(self.style(id), 0, nrows, 0, ncols, 0);
        let mut ri = 0;
        while let Some(&g) = groups.get(ri) {
            let n = groups
                .iter()
                .skip(ri)
                .take_while(|&&x| x == g)
                .count()
                .max(1);
            if let Some(g) = g {
                offer(self.style(g), ri, ri.saturating_add(n), 0, ncols, 1);
            }
            ri = ri.saturating_add(n);
        }
        for (r, &row) in rows.iter().enumerate() {
            offer(self.style(row), r, r.saturating_add(1), 0, ncols, 2);
        }
        for c in cells.iter().filter(|c| c.anon.is_empty()) {
            offer(
                self.style(c.node),
                c.row,
                c.row.saturating_add(c.rs),
                c.col,
                c.col.saturating_add(c.cs).min(ncols),
                3,
            );
        }
        // A row's border does not cross a cell that spans it.
        for c in cells.iter() {
            let (r1, c1) = (
                c.row.saturating_add(c.rs),
                c.col.saturating_add(c.cs).min(ncols),
            );
            for r in c.row.saturating_add(1)..r1 {
                for k in c.col..c1 {
                    if let Some(l) = above.get_mut(at_above(r, k)) {
                        *l = Line::default();
                    }
                }
            }
            for r in c.row..r1 {
                for k in c.col.saturating_add(1)..c1 {
                    if let Some(l) = left.get_mut(at_left(r, k)) {
                        *l = Line::default();
                    }
                }
            }
        }
        for c in cells.iter_mut().filter(|c| c.anon.is_empty()) {
            let (r1, c1) = (
                c.row.saturating_add(c.rs),
                c.col.saturating_add(c.cs).min(ncols),
            );
            let w_above = |r: usize| {
                (c.col..c1)
                    .filter_map(|k| above.get(at_above(r, k)))
                    .fold(0.0f32, |a, l| a.max(l.w))
            };
            let w_left = |k: usize| {
                (c.row..r1)
                    .filter_map(|r| left.get(at_left(r, k)))
                    .fold(0.0f32, |a, l| a.max(l.w))
            };
            c.ins = [
                w_above(c.row) / 2.0,
                w_left(c1) / 2.0,
                w_above(r1) / 2.0,
                w_left(c.col) / 2.0,
            ];
            let (h, v) = (c.ins[1] + c.ins[3], c.ins[0] + c.ins[2]);
            c.hb += h;
            c.vb += v;
            c.min += h;
            c.max += h;
            if let Len::Px(w) = &mut c.spec {
                *w += h;
            }
        }
        let widest = |it: &mut dyn Iterator<Item = &Line>| it.fold(0.0f32, |a, l| a.max(l.w)) / 2.0;
        let edge = [
            widest(&mut above.iter().take(ncols)),
            widest(&mut left.iter().skip(ncols).step_by(ncols.saturating_add(1))),
            widest(&mut above.iter().skip(nrows.saturating_mul(ncols)).take(ncols)),
            widest(&mut left.iter().step_by(ncols.saturating_add(1))),
        ];
        Collapsed { above, left, edge }
    }

    /// Paint the collapsed lines of a table whose grid starts at (`x`, `y`).
    fn collapsed_lines(&mut self, co: &Collapsed, x: f32, y: f32, colx: &[f32], rowy: &[f32]) {
        let ncols = colx.len().saturating_sub(1);
        let nrows = rowy.len().saturating_sub(1);
        let pos = |v: &[f32], k: usize| v.get(k).copied().unwrap_or(0.0);
        // Up the lines that meet at a corner, so the other direction joins them.
        let vert = |r: usize, c: usize| {
            co.left
                .get(r.saturating_mul(ncols.saturating_add(1)).saturating_add(c))
                .map_or(0.0, |l| l.w)
        };
        let horiz = |r: usize, c: usize| {
            co.above
                .get(r.saturating_mul(ncols).saturating_add(c))
                .map_or(0.0, |l| l.w)
        };
        for r in 0..=nrows {
            let row = r.saturating_mul(ncols);
            let mut c = 0;
            while c < ncols {
                let Some(&l) = co.above.get(row.saturating_add(c)) else {
                    break;
                };
                let mut end = c.saturating_add(1);
                while end < ncols
                    && co
                        .above
                        .get(row.saturating_add(end))
                        .is_some_and(|m| m.w == l.w && m.color == l.color)
                {
                    end = end.saturating_add(1);
                }
                if l.w > 0.0 && l.color[3] > 0 {
                    let joint = |k: usize| {
                        let up = r.checked_sub(1).map_or(0.0, |u| vert(u, k));
                        up.max(vert(r, k)) / 2.0
                    };
                    let x0 = pos(colx, c) - joint(c);
                    let x1 = pos(colx, end) + joint(end);
                    self.out.push(Item::Rect {
                        r: Rect {
                            x: x + x0,
                            y: y + pos(rowy, r) - l.w / 2.0,
                            w: fin(x1 - x0),
                            h: l.w,
                        },
                        color: l.color,
                        radius: Radius::NONE,
                    });
                }
                c = end;
            }
        }
        for c in 0..=ncols {
            let mut r = 0;
            while r < nrows {
                let Some(&l) = co
                    .left
                    .get(r.saturating_mul(ncols.saturating_add(1)).saturating_add(c))
                else {
                    break;
                };
                let mut end = r.saturating_add(1);
                while end < nrows
                    && co
                        .left
                        .get(
                            end.saturating_mul(ncols.saturating_add(1))
                                .saturating_add(c),
                        )
                        .is_some_and(|m| m.w == l.w && m.color == l.color)
                {
                    end = end.saturating_add(1);
                }
                if l.w > 0.0 && l.color[3] > 0 {
                    let joint = |k: usize| {
                        let before = c.checked_sub(1).map_or(0.0, |u| horiz(k, u));
                        before.max(if c < ncols { horiz(k, c) } else { 0.0 }) / 2.0
                    };
                    let y0 = pos(rowy, r) - joint(r);
                    let y1 = pos(rowy, end) + joint(end);
                    self.out.push(Item::Rect {
                        r: Rect {
                            x: x + pos(colx, c) - l.w / 2.0,
                            y: y + y0,
                            w: l.w,
                            h: fin(y1 - y0),
                        },
                        color: l.color,
                        radius: Radius::NONE,
                    });
                }
                r = end;
            }
        }
    }

    fn spacing(&self, id: Id) -> f32 {
        let s = self.style(id);
        if s.collapse { 0.0 } else { s.spacing }
    }

    /// What the table's own box takes around its grid: top, right, bottom,
    /// left. Padding does not apply to a collapsed table, whose border is
    /// half of the outer lines.
    fn table_edges(&self, id: Id, ti: &TableInfo) -> [f32; 4] {
        let s = self.style(id);
        if s.collapse {
            return ti.collapsed.as_ref().map_or([0.0; 4], |c| c.edge);
        }
        let mut e = s.border;
        for (e, p) in e.iter_mut().zip(s.padding) {
            *e += p;
        }
        e
    }

    fn table_extra(&self, id: Id, ti: &TableInfo) -> f32 {
        let e = self.table_edges(id, ti);
        self.spacing(id) * (ti.ncols as f32 + 1.0) + e[1] + e[3]
    }

    pub fn table_intrinsic(&mut self, id: Id) -> (f32, f32) {
        if let Some(&r) = self.widths.get(&id) {
            return r;
        }
        let ti = self.table_info(id);
        let (smin, smax) = (
            ti.col_min.iter().sum::<f32>(),
            ti.col_max.iter().sum::<f32>(),
        );
        let extra = self.table_extra(id, &ti);
        self.tables.insert(id, ti);
        let mut r = (fin(smin + extra), fin(smax + extra));
        if let Len::Px(w) = self.style(id).width {
            r.1 = w.min(self.maxw).max(r.0);
        }
        r = (r.0.min(self.maxw), r.1.min(self.maxw));
        self.widths.insert(id, r);
        r
    }

    pub fn table_width(&mut self, id: Id, avail: f32) -> f32 {
        let (mn, mx) = self.table_intrinsic(id);
        let s = self.style(id);
        let mut w = match s.width {
            Len::Px(p) => p,
            Len::Pct(p) => avail * p / 100.0,
            Len::Auto => mx.min(avail),
        };
        if let Len::Px(m) = s.max_width {
            w = w.min(m);
        }
        // A fixed table takes its width whatever is inside.
        if s.fixed_layout && s.width != Len::Auto {
            return fin(w).min(self.maxw);
        }
        fin(w.max(mn)).min(self.maxw)
    }

    /// The `<caption>`s of table `id`, stacked over the width `tw` at `y`.
    /// Returns their height.
    fn captions(&mut self, id: Id, x: f32, y: f32, tw: f32) -> f32 {
        let caps: Vec<Id> = self
            .dom
            .children(id)
            .iter()
            .copied()
            .filter(|&c| self.dom.tag(c) == "caption" && self.style(c).display != Display::None)
            .collect();
        let mut h = 0.0;
        for c in caps {
            h += self.box_at(c, x, y + h, tw, false).h;
        }
        h
    }

    pub fn table_layout(&mut self, id: Id, x: f32, y: f32, tw: f32) -> f32 {
        let cap = self.captions(id, x, y, tw);
        cap + self.table_grid(id, x, y + cap, tw)
    }

    fn table_grid(&mut self, id: Id, x: f32, y: f32, tw: f32) -> f32 {
        let s = self.style(id).clone();
        let sp = self.spacing(id);
        let ti = self.table_info(id);
        let ncols = ti.ncols;
        let edges = self.table_edges(id, &ti);
        let bx = edges[3] + sp;
        let avail = fin(tw - self.table_extra(id, &ti));
        // Column widths: preferred, then shrunk towards min or grown.
        let mut colw: Vec<f32> = (0..ncols)
            .map(|k| {
                let mn = ti.col_min.get(k).copied().unwrap_or(0.0);
                match ti.col_spec.get(k) {
                    Some(Len::Px(p)) => p.max(mn),
                    Some(Len::Pct(p)) => (avail * p / 100.0).max(mn),
                    _ => ti.col_max.get(k).copied().unwrap_or(0.0),
                }
            })
            .collect();
        if s.fixed_layout && s.width != Len::Auto {
            colw = self.fixed_columns(id, &ti, avail);
        }
        let total: f32 = colw.iter().sum();
        if total > avail {
            let shrinkable: f32 = colw.iter().zip(&ti.col_min).map(|(p, m)| p - m).sum();
            if shrinkable > 0.0 {
                let f = ((total - avail) / shrinkable).min(1.0);
                for (w, m) in colw.iter_mut().zip(&ti.col_min) {
                    *w -= (*w - m) * f;
                }
            }
        } else if total < avail && ncols > 0 {
            let grow = avail - total;
            let is_auto = |k: usize| matches!(ti.col_spec.get(k), Some(Len::Auto));
            let weight = |k: usize| ti.col_max.get(k).copied().unwrap_or(0.0).max(1.0);
            let auto_w: f32 = (0..ncols).filter(|&k| is_auto(k)).map(weight).sum();
            if auto_w > 0.0 {
                for (k, w) in colw.iter_mut().enumerate() {
                    if is_auto(k) {
                        *w += grow * weight(k) / auto_w;
                    }
                }
            } else {
                let all: f32 = colw.iter().map(|p| p.max(1.0)).sum();
                for w in colw.iter_mut() {
                    *w += grow * w.max(1.0) / all;
                }
            }
        }
        let mut colx = vec![0.0f32; ncols.saturating_add(1)];
        let mut acc = 0.0;
        for (cx, w) in colx.iter_mut().skip(1).zip(&colw) {
            acc += w + sp;
            *cx = acc;
        }
        let colx_at = |k: usize| colx.get(k).copied().unwrap_or(acc);
        // Lay cells out into captured item lists at their final widths.
        let nrows = ti.rows.len();
        let mut row_h = vec![0.0f32; nrows];
        let mut placed: Vec<(Vec<Item>, f32, f32)> = Vec::with_capacity(ti.cells.len());
        for c in &ti.cells {
            if self.expired() {
                break;
            }
            let w = colx_at(c.col.saturating_add(c.cs)) - colx_at(c.col) - sp;
            let cw = fin(w - c.hb);
            let saved = std::mem::take(&mut self.out);
            let ch = if c.anon.is_empty() {
                let kids = self.dom.children(c.node).to_vec();
                self.block_list(c.node, &kids, 0.0, 0.0, cw)
            } else {
                self.block_list(c.node, &c.anon, 0.0, 0.0, cw)
            };
            let items = std::mem::replace(&mut self.out, saved);
            let mut h = ch + c.vb;
            if c.anon.is_empty() {
                let s = self.style(c.node);
                let extra = if s.border_box { 0.0 } else { c.vb };
                // A cell's height is a minimum, and its content box's unless
                // `box-sizing` says otherwise.
                for spec in [s.height, s.min_height] {
                    if let Len::Px(hh) = spec {
                        h = h.max(hh + extra);
                    }
                }
            }
            if c.rs == 1
                && let Some(rh) = row_h.get_mut(c.row)
            {
                *rh = rh.max(h);
            }
            placed.push((items, ch, h));
        }
        for (rh, &r) in row_h.iter_mut().zip(&ti.rows) {
            if let Len::Px(hh) = self.style(r).height {
                *rh = rh.max(hh);
            }
        }
        for (c, (_, _, h)) in ti.cells.iter().zip(&placed) {
            if c.rs > 1 {
                let last = c.row.saturating_add(c.rs).min(nrows).saturating_sub(1);
                let rows = row_h.get(c.row..=last).unwrap_or(&[]);
                let have: f32 = rows.iter().sum::<f32>() + sp * rows.len().saturating_sub(1) as f32;
                if let Some(rh) = row_h.get_mut(last).filter(|_| *h > have) {
                    *rh += h - have;
                }
            }
        }
        let mut rowy = vec![0.0f32; nrows.saturating_add(1)];
        let mut acc = 0.0;
        for (ry, rh) in rowy.iter_mut().skip(1).zip(&row_h) {
            acc += rh + sp;
            *ry = acc;
        }
        let rowy_at = |k: usize| rowy.get(k).copied().unwrap_or(acc);
        let by = edges[0] + sp;
        let th = if nrows == 0 {
            edges[0] + edges[2] + sp
        } else {
            by + acc + edges[2]
        };
        // Backgrounds: table, row groups, rows.
        let table_r = Rect { x, y, w: tw, h: th };
        if s.bg[3] > 0 {
            self.out.push(Item::Rect {
                r: table_r,
                color: s.bg,
                radius: s.radius_on(tw),
            });
        }
        if let Some(back) = self.background(&s, table_r) {
            self.out.push(back);
        }
        let row_w = avail + sp * (ncols as f32 - 1.0).max(0.0);
        let mut seen_group: Vec<Id> = vec![];
        for (ri, &r) in ti.rows.iter().enumerate() {
            if let Some(Some(g)) = ti.groups.get(ri)
                && !seen_group.contains(g)
                && self.style(*g).bg[3] > 0
            {
                seen_group.push(*g);
                let first = ri;
                let last = ti.groups.iter().rposition(|x| *x == Some(*g)).unwrap_or(ri);
                let r = Rect {
                    x: x + bx,
                    y: y + by + rowy_at(first),
                    w: row_w,
                    h: rowy_at(last.saturating_add(1)) - rowy_at(first) - sp,
                };
                self.out.push(Item::Rect {
                    r,
                    color: self.style(*g).bg,
                    radius: Radius::NONE,
                });
            }
            let color = self.style(r).bg;
            if color[3] > 0 {
                let r = Rect {
                    x: x + bx,
                    y: y + by + rowy_at(ri),
                    w: row_w,
                    h: row_h.get(ri).copied().unwrap_or(0.0),
                };
                self.out.push(Item::Rect {
                    r,
                    color,
                    radius: Radius::NONE,
                });
            }
        }
        for (c, (mut items, content_h, _)) in ti.cells.iter().zip(placed) {
            let cs = if c.anon.is_empty() {
                self.style(c.node).clone()
            } else {
                let mut d = self.default.clone();
                d.valign = VAlign::Top;
                d
            };
            let last = c.row.saturating_add(c.rs).min(nrows).saturating_sub(1);
            let cell_h = rowy_at(last.saturating_add(1)) - rowy_at(c.row) - sp;
            let cell_w = colx_at(c.col.saturating_add(c.cs)) - colx_at(c.col) - sp;
            let cx = if s.rtl {
                x + tw - bx - cell_w - colx_at(c.col)
            } else {
                x + bx + colx_at(c.col)
            };
            let cy = y + by + rowy_at(c.row);
            let r = Rect {
                x: cx,
                y: cy,
                w: cell_w,
                h: cell_h,
            };
            if cs.bg[3] > 0 {
                self.out.push(Item::Rect {
                    r,
                    color: cs.bg,
                    radius: cs.radius_on(cell_w),
                });
            }
            if let Some(back) = self.background(&cs, r) {
                self.out.push(back);
            }
            if !s.collapse {
                self.borders(cx, cy, cell_w, cell_h, &cs);
            }
            // The content, not a declared height, moves within the cell.
            let inner_h = cell_h - c.vb;
            let dy = match cs.valign {
                VAlign::Top | VAlign::Baseline => 0.0,
                VAlign::Middle => fin((inner_h - content_h) / 2.0),
                VAlign::Bottom => fin(inner_h - content_h),
            };
            let (ox, oy) = if c.anon.is_empty() {
                (c.ins[3] + cs.padding[3], c.ins[0] + cs.padding[0])
            } else {
                (0.0, 0.0)
            };
            shift(&mut items, cx + ox, cy + oy + dy);
            self.out.append(&mut items);
        }
        if s.collapse {
            if let Some(co) = &ti.collapsed {
                self.collapsed_lines(co, x + bx, y + by, &colx, &rowy);
            }
        } else {
            self.borders(x, y, tw, th, &s);
        }
        self.tables.insert(id, ti);
        th
    }
}
