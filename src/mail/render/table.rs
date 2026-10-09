//! Tables: the auto layout algorithm with colspan and rowspan, cell
//! spacing, collapsed borders and row/row-group backgrounds. Children of a
//! row that are not cells (a `td { display: block }` media rule) become one
//! anonymous cell whose content stacks as blocks, as CSS specifies.

use super::dom::Id;
use super::layout::{Item, Layouter, Rect, fin, shift};
use super::style::{Display, Len, VAlign};

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
    /// Grid occupancy per row.
    occ: Vec<Vec<bool>>,
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
                    let hb = s.padding[1] + s.padding[3] + s.border[1] + s.border[3];
                    let vb = s.padding[0] + s.padding[2] + s.border[0] + s.border[2];
                    let (mn, mx) = match s.width {
                        Len::Px(w) => {
                            let w = w.min(self.maxw);
                            (mn.min(w.max(mn)), w.max(mn))
                        }
                        _ => (mn, mx),
                    };
                    ((s.width, hb, vb), mn, mx)
                } else {
                    let (mn, mx) = self.list_widths(node, &anon);
                    ((Len::Auto, 0.0, 0.0), mn, mx)
                };
                let (spec, hb, vb) = cell;
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
                });
                col = end;
            }
        }
        let ncols = occ.iter().map(Vec::len).max().unwrap_or(0);
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
            occ,
        }
    }

    fn spacing(&self, id: Id) -> f32 {
        let s = self.style(id);
        if s.collapse { 0.0 } else { s.spacing }
    }

    fn table_extra(&self, id: Id, ncols: usize) -> f32 {
        let s = self.style(id);
        self.spacing(id) * (ncols as f32 + 1.0) + s.border[1] + s.border[3]
    }

    pub fn table_intrinsic(&mut self, id: Id) -> (f32, f32) {
        if let Some(&r) = self.widths.get(&id) {
            return r;
        }
        let ti = self.table_info(id);
        let (ncols, smin, smax) = (
            ti.ncols,
            ti.col_min.iter().sum::<f32>(),
            ti.col_max.iter().sum::<f32>(),
        );
        self.tables.insert(id, ti);
        let extra = self.table_extra(id, ncols);
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
        fin(w.max(mn)).min(self.maxw)
    }

    pub fn table_layout(&mut self, id: Id, x: f32, y: f32, tw: f32) -> f32 {
        let s = self.style(id).clone();
        let sp = self.spacing(id);
        let ti = self.table_info(id);
        let ncols = ti.ncols;
        let bx = s.border[3] + sp;
        let avail = fin(tw - self.table_extra(id, ncols));
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
        let mut placed: Vec<(Vec<Item>, f32)> = Vec::with_capacity(ti.cells.len());
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
            if let (true, Len::Px(hh)) = (c.anon.is_empty(), self.style(c.node).height) {
                h = h.max(hh);
            }
            if c.rs == 1
                && let Some(rh) = row_h.get_mut(c.row)
            {
                *rh = rh.max(h);
            }
            placed.push((items, h));
        }
        for (rh, &r) in row_h.iter_mut().zip(&ti.rows) {
            if let Len::Px(hh) = self.style(r).height {
                *rh = rh.max(hh);
            }
        }
        for (c, (_, h)) in ti.cells.iter().zip(&placed) {
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
        let by = s.border[0] + sp;
        let th = if nrows == 0 {
            s.border[0] + s.border[2] + sp
        } else {
            by + acc + s.border[2]
        };
        // Backgrounds: table, row groups, rows.
        let table_r = Rect { x, y, w: tw, h: th };
        if s.bg[3] > 0 {
            self.out.push(Item::Rect {
                r: table_r,
                color: s.bg,
                radius: s.radius,
            });
        }
        if let Some(ix) = self.load_bg(&s) {
            self.out.push(Item::Image {
                r: table_r,
                img: ix,
                clip: Some(table_r),
                radius: 0.0,
            });
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
                    radius: 0.0,
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
                    radius: 0.0,
                });
            }
        }
        for (c, (mut items, h)) in ti.cells.iter().zip(placed) {
            let mut cs = if c.anon.is_empty() {
                self.style(c.node).clone()
            } else {
                let mut d = self.default.clone();
                d.valign = VAlign::Top;
                d
            };
            let last = c.row.saturating_add(c.rs).min(nrows).saturating_sub(1);
            let cell_h = rowy_at(last.saturating_add(1)) - rowy_at(c.row) - sp;
            let cell_w = colx_at(c.col.saturating_add(c.cs)) - colx_at(c.col) - sp;
            let cx = x + bx + colx_at(c.col);
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
                    radius: cs.radius,
                });
            }
            if let Some(ix) = self.load_bg(&cs) {
                self.out.push(Item::Image {
                    r,
                    img: ix,
                    clip: Some(r),
                    radius: 0.0,
                });
            }
            if s.collapse {
                // The neighbour on the left / above owns the shared border.
                let taken = |row: Option<usize>, col: Option<usize>| -> bool {
                    match (row, col) {
                        (Some(row), Some(col)) => ti
                            .occ
                            .get(row)
                            .and_then(|o| o.get(col))
                            .copied()
                            .unwrap_or(false),
                        _ => false,
                    }
                };
                if taken(Some(c.row), c.col.checked_sub(1)) {
                    cs.border[3] = 0.0;
                }
                if taken(c.row.checked_sub(1), Some(c.col)) {
                    cs.border[0] = 0.0;
                }
            }
            self.borders(cx, cy, cell_w, cell_h, &cs);
            let content_h = h - c.vb;
            let inner_h = cell_h - c.vb;
            let dy = match cs.valign {
                VAlign::Top | VAlign::Baseline => 0.0,
                VAlign::Middle => fin((inner_h - content_h) / 2.0),
                VAlign::Bottom => fin(inner_h - content_h),
            };
            let (ox, oy) = if c.anon.is_empty() {
                (cs.border[3] + cs.padding[3], cs.border[0] + cs.padding[0])
            } else {
                (0.0, 0.0)
            };
            shift(&mut items, cx + ox, cy + oy + dy);
            self.out.append(&mut items);
        }
        self.borders(x, y, tw, th, &s);
        self.tables.insert(id, ti);
        th
    }
}
