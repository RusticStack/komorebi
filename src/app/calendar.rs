//! The Calendar page: the agenda (30 days), day, week and month views
//! from the local store (`komorebi::cal::store`). Nothing here runs at
//! launch: the first query runs when the page opens (`cal-opened`), on the
//! timer after that frame, and each view reads only its own range with
//! one query (lazily, per period shown).

use super::*;
use crate::{AgendaLine, Cal, CalAllDay, CalBlock, CalCell, CalChip};
use chrono::{
    DateTime, Datelike, Duration as Days, Local, Months, NaiveDate, NaiveTime, TimeZone, Timelike,
};
use komorebi::cal::layout;
use komorebi::cal::remind::{Marker, marker};
use komorebi::cal::{FLAG_ALL_DAY, store::AgendaRow};

/// Days the agenda shows at once.
const SPAN_DAYS: i64 = 30;
/// Events a month cell holds at most (the rest is "+N more").
const CHIPS: usize = 4;
/// The views (`Cal.view`).
pub(super) const AGENDA: i32 = 0;
pub(super) const DAY: i32 = 1;
pub(super) const WEEK: i32 = 2;
pub(super) const MONTH: i32 = 3;

thread_local! {
    /// The agenda's first day (local midnight, unix seconds); 0 for today.
    static FROM: Cell<i64> = const { Cell::new(0) };
    /// The day the other views are about (the selected day).
    static ANCHOR: Cell<Option<NaiveDate>> = const { Cell::new(None) };
    /// The occurrences shown, (event, start): the `key` of a chip or line.
    pub(super) static KEYS: RefCell<Vec<(i64, i64)>> = const { RefCell::new(Vec::new()) };
    /// The month grid's first day.
    static GRID: Cell<Option<NaiveDate>> = const { Cell::new(None) };
    /// Snapshots: open this many seconds' event in the editor after showing.
    pub(super) static SAMPLE_EDIT: Cell<bool> = const { Cell::new(false) };
}

/// Local midnight of the day holding `ts`.
fn midnight(ts: i64) -> Option<DateTime<Local>> {
    let day = Local.timestamp_opt(ts, 0).single()?.date_naive();
    day_start(day)
}

/// Local midnight of `day` (the first instant of it after a gap).
pub(super) fn day_start(day: NaiveDate) -> Option<DateTime<Local>> {
    Local
        .from_local_datetime(&day.and_time(NaiveTime::MIN))
        .earliest()
        .or_else(|| {
            Local
                .from_local_datetime(&day.and_hms_opt(1, 0, 0)?)
                .earliest()
        })
}

fn today() -> NaiveDate {
    Local::now().date_naive()
}

pub(super) fn set_anchor(d: NaiveDate) {
    ANCHOR.set(Some(d));
}

pub(super) fn anchor() -> NaiveDate {
    ANCHOR.get().unwrap_or_else(today)
}

/// The first day of the week holding `d`.
fn week_start(d: NaiveDate) -> NaiveDate {
    let first = format::first_weekday();
    let back = (7 + d.weekday().num_days_from_monday() - first.num_days_from_monday()) % 7;
    d - Days::days(i64::from(back))
}

/// A key for an occurrence (its index in [`KEYS`]).
fn key(keys: &mut Vec<(i64, i64)>, event: i64, start: i64) -> i32 {
    keys.push((event, start));
    i32::try_from(keys.len() - 1).unwrap_or(-1)
}

/// The wall-clock minutes of an instant from its own midnight.
fn wall_min(t: i64) -> i64 {
    Local
        .timestamp_opt(t, 0)
        .single()
        .map_or(0, |d| i64::from(d.hour() * 60 + d.minute()))
}

fn title_of(r: &AgendaRow) -> SharedString {
    if r.summary.is_empty() {
        t!(invoke_invite_no_title).into()
    } else {
        r.summary.as_str().into()
    }
}

impl App {
    /// The page opened: show the view once the frame is drawn.
    pub(super) fn calendar_opened(&mut self) {
        FROM.set(0);
        ANCHOR.set(None);
        Timer::single_shot(Duration::ZERO, || {
            with_app(|a| {
                // A new month or another time zone: place the occurrences
                // again first (local-only calendars: a few ms).
                let now = chrono::Utc::now().timestamp();
                if let Some(key) = komorebi::cal::window_key(now) {
                    a.db.refresh_occurrences(&key, now).ok();
                }
                restore_view(a);
                a.calendar_show();
                a.calendar_kick();
                a.arm_reminders();
                if SAMPLE_EDIT.replace(false) {
                    a.calendar_sample_edit();
                }
            });
        });
    }

    /// The store changed (a sync pass, a calendar turned on or off).
    pub(super) fn calendar_reload(&mut self) {
        if self.ui().get_show_calendar() {
            self.calendar_show();
        }
        self.arm_reminders();
    }

    /// Today, or the agenda's 30 days earlier or later (`days`).
    pub(super) fn calendar_move(&mut self, days: i64) {
        let from = match FROM.get() {
            0 => chrono::Utc::now().timestamp(),
            f => f,
        };
        FROM.set(if days == 0 {
            0
        } else {
            from.saturating_add(days.saturating_mul(86_400))
        });
        self.calendar_show();
    }

    /// Previous (-1), today (0) or next (1) period of the view shown.
    fn calendar_step(&mut self, dir: i32) {
        let view = self.ui().global::<Cal>().get_view();
        if dir == 0 {
            ANCHOR.set(None);
            return self.calendar_move(0);
        }
        let a = anchor();
        let d = i64::from(dir.signum());
        let next = match view {
            AGENDA => return self.calendar_move(d * SPAN_DAYS),
            DAY => a.checked_add_signed(Days::days(d)),
            WEEK => a.checked_add_signed(Days::days(7 * d)),
            _ if d > 0 => a.checked_add_months(Months::new(1)),
            _ => a.checked_sub_months(Months::new(1)),
        };
        ANCHOR.set(next.or(Some(a)));
        self.calendar_show();
    }

    /// Show the view (`Cal.view`) for the anchor day.
    pub(super) fn calendar_show(&mut self) {
        let ui = self.ui();
        let g = ui.global::<Cal>();
        g.set_conflicts(self.db.conflicts().unwrap_or(0).clamp(0, 999) as i32);
        let wd = format::first_weekday();
        let names: Vec<SharedString> = (0..7)
            .map(|i| {
                let mut w = wd;
                for _ in 0..i {
                    w = w.succ();
                }
                format::weekday_short(self.lang, w).into()
            })
            .collect();
        g.set_weekdays(ModelRc::new(VecModel::from(names)));
        KEYS.with_borrow_mut(Vec::clear);
        match g.get_view() {
            DAY => self.show_grid(anchor(), 1),
            WEEK => self.show_grid(week_start(anchor()), 7),
            MONTH => self.show_month(),
            _ => self.show_agenda(),
        }
        ui.set_cal_loaded(true);
    }

    fn show_agenda(&mut self) {
        let ui = self.ui();
        let now = Local::now();
        let start = match FROM.get() {
            0 => now.timestamp(),
            f => f,
        };
        let Some(from) = midnight(start) else {
            return;
        };
        let to = from
            .checked_add_signed(Days::days(SPAN_DAYS))
            .unwrap_or(from);
        let rows = self
            .db
            .agenda(from.timestamp(), to.timestamp())
            .unwrap_or_default();
        let lines = agenda_lines(self.lang, &rows, from, SPAN_DAYS, &now);
        let last = to.checked_sub_signed(Days::days(1)).unwrap_or(to);
        ui.set_cal_range(
            t!(
                invoke_cal_range,
                format::range_day(self.lang, &from).into(),
                format::range_day(self.lang, &last).into()
            )
            .into(),
        );
        ui.set_cal_lines(ModelRc::new(VecModel::from(lines)));
    }

    /// The month grid's cells for the month of `day` (`chips`: with the
    /// events; the date picker has none), and its first day.
    pub(super) fn month_cells(
        &self,
        day: NaiveDate,
        selected: NaiveDate,
        chips: bool,
    ) -> (Vec<CalCell>, NaiveDate) {
        let first = day.with_day(1).unwrap_or(day);
        let start = week_start(first);
        let lo = day_start(start).map_or(0, |t| t.timestamp());
        let hi = day_start(start + Days::days(42)).map_or(0, |t| t.timestamp());
        let rows = if chips {
            self.db.agenda(lo, hi).unwrap_or_default()
        } else {
            Vec::new()
        };
        let t = today();
        let mut cells = Vec::with_capacity(42);
        KEYS.with_borrow_mut(|keys| {
            for i in 0..42 {
                let d = start + Days::days(i);
                let (a, b) = (
                    day_start(d).map_or(0, |t| t.timestamp()),
                    day_start(d + Days::days(1)).map_or(0, |t| t.timestamp()),
                );
                let mut of_day: Vec<&AgendaRow> = rows
                    .iter()
                    .filter(|r| r.start < b && r.end.max(r.start + 1) > a)
                    .collect();
                // All-day and longer events first, as Windows' calendar.
                of_day.sort_by_key(|r| (r.flags & FLAG_ALL_DAY == 0 && r.start >= a, r.start));
                let list: Vec<CalChip> = of_day
                    .iter()
                    .take(CHIPS)
                    .map(|r| CalChip {
                        title: title_of(r),
                        time: if r.flags & FLAG_ALL_DAY != 0 || r.start < a {
                            SharedString::new()
                        } else {
                            format::time(r.start).into()
                        },
                        key: key(keys, r.event_id, r.start),
                        cancelled: r.cancelled,
                    })
                    .collect();
                let local = day_start(d).unwrap_or_else(Local::now);
                cells.push(CalCell {
                    day: d.day().to_string().into(),
                    label: format::day_title(self.lang, &local).into(),
                    in_month: d.month() == first.month(),
                    today: d == t,
                    selected: d == selected,
                    count: of_day.len().min(999) as i32,
                    chips: ModelRc::new(VecModel::from(list)),
                });
            }
        });
        (cells, start)
    }

    fn show_month(&mut self) {
        let a = anchor();
        let (cells, start) = self.month_cells(a, a, true);
        GRID.set(Some(start));
        let ui = self.ui();
        let g = ui.global::<Cal>();
        g.set_title(format::month_title(self.lang, &a).into());
        g.set_cells(ModelRc::new(VecModel::from(cells)));
        // The selected day's agenda beside (or under) the grid.
        let now = Local::now();
        let lines = match day_start(a) {
            Some(from) => {
                let to = day_start(a + Days::days(1)).unwrap_or(from);
                let rows = self
                    .db
                    .agenda(from.timestamp(), to.timestamp())
                    .unwrap_or_default();
                let mut l = agenda_lines(self.lang, &rows, from, 1, &now);
                if l.is_empty() {
                    l.push(AgendaLine {
                        header: true,
                        when: format::day_title(self.lang, &from).into(),
                        ..Default::default()
                    });
                }
                l
            }
            None => Vec::new(),
        };
        g.set_day_lines(ModelRc::new(VecModel::from(lines)));
    }

    /// The time grid for `days` days from `first`.
    fn show_grid(&mut self, first: NaiveDate, days: i64) {
        let lang = self.lang;
        let starts: Vec<i64> = (0..=days)
            .map(|i| day_start(first + Days::days(i)).map_or(0, |t| t.timestamp()))
            .collect();
        let (lo, hi) = (starts[0], starts[days as usize]);
        let rows = self.db.agenda(lo, hi).unwrap_or_default();
        let mut blocks = Vec::new();
        let mut strip: Vec<CalAllDay> = Vec::new();
        let mut first_top = i64::MAX;
        KEYS.with_borrow_mut(|keys| {
            // All-day events and those of a day or more: the strip, each
            // in the first row free over its days.
            let mut rows_end: Vec<i64> = Vec::new();
            for r in rows
                .iter()
                .filter(|r| r.flags & FLAG_ALL_DAY != 0 || r.end - r.start >= 86_400)
            {
                let col = |t: i64| starts.iter().rposition(|&s| s <= t).unwrap_or(0) as i64;
                let a = col(r.start.max(lo)).min(days - 1);
                let b = col((r.end - 1).max(r.start).min(hi - 1)).min(days - 1);
                let row = match rows_end.iter().position(|&e| e < a) {
                    Some(i) => i,
                    None => {
                        rows_end.push(-1);
                        rows_end.len() - 1
                    }
                };
                rows_end[row] = b;
                strip.push(CalAllDay {
                    day: a as i32,
                    span: (b - a + 1) as i32,
                    row: row as i32,
                    title: title_of(r),
                    key: key(keys, r.event_id, r.start),
                });
            }
            for d in 0..days as usize {
                let (a, b) = (starts[d], starts[d + 1]);
                let timed: Vec<&AgendaRow> = rows
                    .iter()
                    .filter(|r| r.flags & FLAG_ALL_DAY == 0 && r.end - r.start < 86_400)
                    .filter(|r| layout::clip(r.start, r.end, a, b, &wall_min).is_some())
                    .collect();
                let spans: Vec<(i64, i64)> = timed
                    .iter()
                    .filter_map(|r| layout::clip(r.start, r.end, a, b, &wall_min))
                    .collect();
                for p in layout::columns(&spans) {
                    let Some(r) = timed.get(p.index) else {
                        continue;
                    };
                    first_top = first_top.min(p.top);
                    blocks.push(CalBlock {
                        day: d as i32,
                        top: p.top as f32,
                        bottom: p.bottom as f32,
                        col: p.col as i32,
                        cols: p.cols as i32,
                        title: title_of(r),
                        time: format!("{}-{}", format::time(r.start), format::time(r.end)).into(),
                        key: key(keys, r.event_id, r.start),
                        cancelled: r.cancelled,
                    });
                }
            }
        });
        let now = Local::now();
        let t = today();
        let today_col = (0..days).find(|i| first + Days::days(*i) == t);
        let columns: Vec<SharedString> = (0..days)
            .filter_map(|i| day_start(first + Days::days(i)))
            .map(|d| format::column_day(lang, &d).into())
            .collect();
        let ui = self.ui();
        let g = ui.global::<Cal>();
        let title = if days == 1 {
            day_start(first).map_or_else(String::new, |d| format::day_title(lang, &d))
        } else {
            match (day_start(first), day_start(first + Days::days(days - 1))) {
                (Some(a), Some(b)) => t!(
                    invoke_cal_range,
                    format::range_day(lang, &a).into(),
                    format::range_day(lang, &b).into()
                ),
                _ => String::new(),
            }
        };
        g.set_title(title.into());
        g.set_columns(ModelRc::new(VecModel::from(columns)));
        g.set_today_col(today_col.map_or(-1, |c| c as i32));
        g.set_now_min(if today_col.is_some() {
            (now.hour() * 60 + now.minute()) as f32
        } else {
            -1.0
        });
        g.set_scroll_min(first_top.min(8 * 60) as i32);
        g.set_all_day_rows(strip.iter().map(|s| s.row + 1).max().unwrap_or(0).min(4));
        g.set_all_day(ModelRc::new(VecModel::from(strip)));
        g.set_blocks(ModelRc::new(VecModel::from(blocks)));
    }

    /// A month cell was clicked: that day is selected (another month's
    /// day moves the grid there).
    fn calendar_pick(&mut self, index: i32) {
        if let Some(start) = GRID.get() {
            ANCHOR.set(Some(start + Days::days(i64::from(index.clamp(0, 41)))));
            self.calendar_show();
        }
    }

    /// Arrow keys in the month: the selected day moves by `days`.
    fn calendar_move_day(&mut self, days: i32) {
        ANCHOR.set(anchor().checked_add_signed(Days::days(i64::from(days))));
        self.calendar_show();
    }

    fn calendar_tick(&mut self) {
        let ui = self.ui();
        let g = ui.global::<Cal>();
        if g.get_today_col() >= 0 {
            let now = Local::now();
            g.set_now_min((now.hour() * 60 + now.minute()) as f32);
        }
    }
}

/// The agenda's lines over `span` days from `from`: a header for each day
/// with events, then the day's occurrences (an event over several days
/// shows on each).
fn agenda_lines(
    lang: komorebi::i18n::Lang,
    rows: &[AgendaRow],
    from: DateTime<Local>,
    span: i64,
    now: &DateTime<Local>,
) -> Vec<AgendaLine> {
    let mut out = Vec::new();
    let today = now.date_naive();
    KEYS.with_borrow_mut(|keys| {
        for d in 0..span {
            let Some(day) = from.checked_add_signed(Days::days(d)) else {
                break;
            };
            let Some(day) = midnight(day.timestamp().saturating_add(3 * 3600)) else {
                continue;
            };
            let next = midnight(day.timestamp().saturating_add(26 * 3600)).unwrap_or(day);
            let (lo, hi) = (day.timestamp(), next.timestamp());
            let mut first = true;
            for r in rows
                .iter()
                .filter(|r| r.start < hi && r.end.max(r.start + 1) > lo)
            {
                if first {
                    let name = format::agenda_day(lang, &day, now);
                    let date = day.date_naive();
                    let when = if date == today {
                        t!(invoke_cal_today, name.into())
                    } else if today.succ_opt() == Some(date) {
                        t!(invoke_cal_tomorrow, name.into())
                    } else {
                        name
                    };
                    out.push(AgendaLine {
                        header: true,
                        when: when.into(),
                        key: -1,
                        ..Default::default()
                    });
                    first = false;
                }
                let all_day = r.flags & FLAG_ALL_DAY != 0;
                let when = if all_day {
                    t!(invoke_cal_all_day)
                } else {
                    // A time before this day or after it: the day's own edge.
                    let s = if r.start < lo { lo } else { r.start };
                    let e = if r.end > hi { hi } else { r.end };
                    if e > s {
                        format!("{}-{}", format::time(s), format::time(e))
                    } else {
                        format::time(s)
                    }
                };
                out.push(AgendaLine {
                    header: false,
                    when: when.into(),
                    title: title_of(r),
                    location: r.location.as_str().into(),
                    recurring: r.recurring,
                    alarm: r.alarm,
                    cancelled: r.cancelled,
                    badge: match marker(now.timestamp(), r.start, r.end, r.alarm_min) {
                        Marker::Now if !all_day => t!(invoke_cal_now).into(),
                        Marker::InMinutes(n) => t!(invoke_cal_in_min, n.min(9_999) as i32).into(),
                        _ => SharedString::new(),
                    },
                    key: key(keys, r.event_id, r.start),
                });
            }
        }
    });
    out
}

pub(super) fn wire(ui: &AppWindow) {
    let g = ui.global::<Cal>();
    g.on_set_view(|v| {
        with_app(|a| {
            a.ui().global::<Cal>().set_view(v.clamp(AGENDA, MONTH));
            a.db.set_setting("cal_view", &v.to_string()).ok();
            a.calendar_show();
        });
    });
    g.on_step(|d| {
        with_app(|a| a.calendar_step(d));
    });
    g.on_pick(|i| {
        with_app(|a| a.calendar_pick(i));
    });
    g.on_move_day(|d| {
        with_app(|a| a.calendar_move_day(d));
    });
    g.on_tick(|| {
        with_app(|a| a.calendar_tick());
    });
    g.on_resolve(|again| {
        with_app(|a| a.calendar_resolve(again));
    });
    super::calendar_edit::wire(ui);
}

/// The view last chosen (`cal_view`), month by default; read when the page
/// first opens.
pub(super) fn restore_view(a: &App) {
    let v =
        a.db.cal_setting("cal_view")
            .and_then(|v| v.parse::<i32>().ok())
            .filter(|v| (AGENDA..=MONTH).contains(v))
            .unwrap_or(MONTH);
    a.ui().global::<Cal>().set_view(v);
}

/// Snapshots (`KOMOREBI_SNAPSHOT_ACTION=calendar:sample`, or `:month`,
/// `:week`, `:day`, `:agenda`, `:edit`; offline on a throwaway cache only,
/// as it writes to it): made-up events in the local calendar (today,
/// tomorrow, an all-day day, a weekly series, overlapping meetings and an
/// accepted invitation), then the Calendar page in that view.
pub fn sample_calendar(view: &str) {
    with_app(|a| {
        let Ok(cal) = a.db.local_calendar() else {
            return;
        };
        let today = Local::now().date_naive();
        let day = |n: i64| {
            today
                .checked_add_signed(Days::days(n))
                .unwrap_or(today)
                .format("%Y%m%d")
                .to_string()
        };
        let now = chrono::Utc::now().timestamp();
        let events = [
            format!(
                "UID:s1\r\nDTSTART:{}T093000\r\nDTEND:{}T100000\r\nSUMMARY:Design review\r\nLOCATION:Room 2\r\n",
                day(0),
                day(0)
            ),
            format!(
                "UID:s2\r\nDTSTART:{}T140000\r\nDTEND:{}T150000\r\nSUMMARY:Dentist\r\nLOCATION:Rua Augusta 10, Lisboa\r\nBEGIN:VALARM\r\nACTION:DISPLAY\r\nTRIGGER:-PT30M\r\nEND:VALARM\r\n",
                day(1),
                day(1)
            ),
            format!(
                "UID:s3\r\nDTSTART;VALUE=DATE:{}\r\nDTEND;VALUE=DATE:{}\r\nSUMMARY:Public holiday\r\n",
                day(3),
                day(4)
            ),
            format!(
                "UID:s4\r\nDTSTART:{}T083000\r\nDTEND:{}T084500\r\nSUMMARY:Team stand-up\r\nRRULE:FREQ=WEEKLY;COUNT=6\r\n",
                day(2),
                day(2)
            ),
            format!(
                "UID:s5\r\nDTSTART:{}T160000\r\nDTEND:{}T170000\r\nSUMMARY:Quarterly planning\r\nORGANIZER;CN=Ana:mailto:ana@example.com\r\nATTENDEE;PARTSTAT=ACCEPTED:mailto:me@example.com\r\nSTATUS:CANCELLED\r\n",
                day(5),
                day(5)
            ),
            format!(
                "UID:s8\r\nDTSTART:{}T100000\r\nDTEND:{}T113000\r\nSUMMARY:Budget call\r\n",
                day(0),
                day(0)
            ),
            format!(
                "UID:s9\r\nDTSTART:{}T110000\r\nDTEND:{}T120000\r\nSUMMARY:Lunch with Rita\r\nRRULE:FREQ=MONTHLY;BYMONTHDAY={}\r\n",
                day(0),
                day(0),
                today.day()
            ),
            format!(
                "UID:s10\r\nDTSTART;VALUE=DATE:{}\r\nDTEND;VALUE=DATE:{}\r\nSUMMARY:Lisbon trip\r\n",
                day(8),
                day(11)
            ),
        ];
        // One under way and one whose reminder is due: the markers.
        let at = |mins: i64| {
            (Local::now() + Days::minutes(mins))
                .format("%Y%m%dT%H%M00")
                .to_string()
        };
        let soon = [
            format!(
                "UID:s6\r\nDTSTART:{}\r\nDTEND:{}\r\nSUMMARY:Focus time\r\n",
                at(-20),
                at(40)
            ),
            format!(
                "UID:s7\r\nDTSTART:{}\r\nDTEND:{}\r\nSUMMARY:Call with Rui\r\nBEGIN:VALARM\r\nACTION:DISPLAY\r\nTRIGGER:-PT15M\r\nEND:VALARM\r\n",
                at(10),
                at(40)
            ),
        ];
        for e in events.into_iter().chain(soon) {
            let raw = format!(
                "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Komorebi//EN\r\nBEGIN:VEVENT\r\n{e}END:VEVENT\r\nEND:VCALENDAR\r\n"
            );
            a.db.save_event(cal, None, &raw, "me@example.com", now).ok();
        }
        let v = match view {
            "agenda" => AGENDA,
            "day" => DAY,
            "week" => WEEK,
            _ => MONTH,
        };
        a.db.set_setting("cal_view", &v.to_string()).ok();
        SAMPLE_EDIT.set(view == "edit");
        a.ui().set_show_calendar(true);
    });
}
