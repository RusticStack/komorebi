//! The event editor (list/details: it takes the Calendar page's content
//! area): create, edit and delete your own events, with the date picker
//! (the month grid in a flyout) and the simple repeat rules of
//! `komorebi::cal::edit`. A series asks whether the change is for this
//! occurrence (an override, or an EXDATE when deleting) or the whole
//! series. An event someone else organized is read only, apart from
//! "Remove from calendar". Changes go to the store at once and, for a
//! CalDAV calendar, to its queue; a pass then sends them.

use super::calendar::{KEYS, anchor, day_start};
use super::*;
use crate::Cal;
use chrono::{Datelike, Duration as Days, Local, Months, NaiveDate, NaiveDateTime, TimeZone};
use komorebi::cal::edit::{self, Form, FormError, Repeat};
use komorebi::cal::store::Calendar;
use komorebi::mail::ical;

/// The reminder choices (`Cal.ed-reminder`), minutes before.
const REMINDERS: [Option<i64>; 7] = [
    None,
    Some(0),
    Some(5),
    Some(15),
    Some(30),
    Some(60),
    Some(1440),
];

#[derive(Default)]
struct Ed {
    /// The event edited (`None`: a new one).
    id: Option<i64>,
    /// The occurrence's RECURRENCE-ID key (empty for a single event).
    key: String,
    /// The occurrence's start as shown, and the series' own start.
    occ: Option<NaiveDateTime>,
    series: Option<NaiveDateTime>,
    /// The calendars offered, by combo index.
    cals: Vec<i64>,
    ics: String,
    /// The event's first alarm is Komorebi's own (else it is kept as is).
    ours_alarm: bool,
    alarm_index: i32,
    /// The organizer's address when it is this account (kept "mine").
    me: String,
    /// The picker's month.
    picker: Option<NaiveDate>,
    picker_start: Option<NaiveDate>,
}

thread_local! {
    static ED: RefCell<Ed> = RefCell::new(Ed::default());
}

fn ed<R>(f: impl FnOnce(&mut Ed) -> R) -> R {
    ED.with(|e| f(&mut e.borrow_mut()))
}

/// Dates are typed day first in Portuguese.
fn day_first(lang: komorebi::i18n::Lang) -> bool {
    lang == komorebi::i18n::Lang::PtPt
}

/// A wall time of the resource in this PC's zone.
fn local_wall(w: &ical::When) -> NaiveDateTime {
    match w.utc {
        Some(t) if !w.all_day => Local
            .timestamp_opt(t, 0)
            .single()
            .map_or(w.wall, |d| d.naive_local()),
        _ => w.wall,
    }
}

impl App {
    fn calendar_names(&self, cals: &[Calendar]) -> Vec<SharedString> {
        cals.iter()
            .map(|c| {
                let account = c
                    .account_id
                    .and_then(|id| self.accounts.iter().find(|a| a.id == id))
                    .map(|a| a.email.clone());
                match account {
                    None => t!(invoke_cal_local_name).into(),
                    Some(acc) => t!(invoke_cal_named, c.name.as_str().into(), acc.into()).into(),
                }
            })
            .collect()
    }

    /// The repeat choices for a series starting on `d`.
    fn fill_repeats(&self, d: NaiveDate) {
        let lang = self.lang;
        let day = format::weekday_full(lang, d.weekday());
        let date = day_start(d).map_or_else(String::new, |t| format::day_month(lang, &t));
        let list: Vec<SharedString> = Repeat::ALL
            .iter()
            .map(|r| match r {
                Repeat::None => t!(invoke_cal_rep_none),
                Repeat::Daily => t!(invoke_cal_rep_daily),
                Repeat::Weekly => t!(invoke_cal_rep_weekly, day.into()),
                Repeat::MonthlyDay => t!(invoke_cal_rep_monthly, d.day() as i32),
                Repeat::MonthlyNth => {
                    t!(invoke_cal_rep_nth, i32::from(edit::nth_of(d)), day.into())
                }
                Repeat::Yearly => t!(invoke_cal_rep_yearly, date.clone().into()),
            })
            .map(SharedString::from)
            .collect();
        self.ui()
            .global::<Cal>()
            .set_ed_repeats(ModelRc::new(VecModel::from(list)));
    }

    fn set_when(&self, start: NaiveDateTime, end: NaiveDateTime, all_day: bool) {
        let ui = self.ui();
        let g = ui.global::<Cal>();
        let df = day_first(self.lang);
        g.set_ed_all_day(all_day);
        g.set_ed_start_date(edit::show_date(start.date(), df).into());
        g.set_ed_end_date(edit::show_date(end.date(), df).into());
        g.set_ed_start_time(start.format("%H:%M").to_string().into());
        g.set_ed_end_time(end.format("%H:%M").to_string().into());
    }

    /// Open the editor: calendars, hint and the common fields reset.
    fn editor_base(&mut self, writable_only: bool) -> Vec<Calendar> {
        let cals: Vec<Calendar> = self
            .db
            .calendars()
            .unwrap_or_default()
            .into_iter()
            .filter(|c| c.enabled && (!writable_only || c.can_write))
            .collect();
        let names = self.calendar_names(&cals);
        let ui = self.ui();
        let g = ui.global::<Cal>();
        g.set_ed_calendars(ModelRc::new(VecModel::from(names)));
        g.set_ed_date_hint(t!(invoke_cal_date_hint, day_first(self.lang)).into());
        g.set_ed_error(SharedString::new());
        g.set_ed_rule(SharedString::new());
        g.set_ed_scope(0);
        g.set_picker(0);
        g.set_ed_read_only(false);
        g.set_ed_series(false);
        g.set_ed_organizer(SharedString::new());
        ed(|e| {
            *e = Ed::default();
            e.cals = cals.iter().map(|c| c.id).collect();
        });
        cals
    }

    /// New event: on the selected day (today: the next hour), an hour
    /// long, in the calendar used last.
    fn calendar_new_event(&mut self) {
        let cals = self.editor_base(true);
        let ui = self.ui();
        let g = ui.global::<Cal>();
        let day = anchor();
        let now = Local::now().naive_local();
        let hour = if day == now.date() {
            (now + Days::hours(1)).format("%H").to_string()
        } else {
            "09".into()
        };
        let start = edit::parse_time(&hour)
            .map_or(day.and_hms_opt(9, 0, 0), |t| Some(day.and_time(t)))
            .unwrap_or_default();
        self.set_when(start, start + Days::hours(1), false);
        let last: Option<i64> = self
            .db
            .cal_setting("cal_default")
            .and_then(|v| v.parse().ok());
        let index = last
            .and_then(|id| cals.iter().position(|c| c.id == id))
            .unwrap_or(0);
        g.set_ed_calendar(index as i32);
        g.set_ed_title(SharedString::new());
        g.set_ed_location(SharedString::new());
        g.set_ed_notes(SharedString::new());
        g.set_ed_repeat(0);
        g.set_ed_reminder(3);
        self.fill_repeats(day);
        g.set_ed_new(true);
        g.set_editing(true);
    }

    /// Open an occurrence (`key` from the view) in the editor.
    fn calendar_open(&mut self, key: i32) {
        let Some((event, start)) =
            KEYS.with_borrow(|k| usize::try_from(key).ok().and_then(|i| k.get(i).copied()))
        else {
            return;
        };
        let Some(info) = self.db.event_info(event).ok().flatten() else {
            return;
        };
        let Some(res) = ical::parse_resource(info.ics.as_bytes()) else {
            return;
        };
        let occ_key = self
            .db
            .occurrence_key(event, start)
            .ok()
            .flatten()
            .unwrap_or_default();
        // This occurrence's own event (an override), else the series'.
        let item = res
            .overrides
            .iter()
            .find(|o| {
                !occ_key.is_empty()
                    && o.event
                        .recurrence_id
                        .trim_end_matches(['Z', 'z'])
                        .ends_with(&occ_key)
            })
            .or(res.master.as_ref())
            .or(res.overrides.first());
        let Some(item) = item else {
            return;
        };
        let read_only = !info.mine;
        let cals = self.editor_base(!read_only);
        let ui = self.ui();
        let g = ui.global::<Cal>();
        let ev = &item.event;
        let all_day = ev.start.all_day;
        // The occurrence's own times in this PC's zone.
        let occ = Local
            .timestamp_opt(start, 0)
            .single()
            .map(|d| d.naive_local());
        let first = local_wall(&ev.start);
        let len = ev
            .end
            .as_ref()
            .map(|e| local_wall(e) - first)
            .unwrap_or_else(|| if all_day { Days::days(1) } else { Days::zero() });
        let s = if all_day {
            occ.map_or(first, |o| o.date().and_time(chrono::NaiveTime::MIN))
        } else {
            occ.unwrap_or(first)
        };
        let mut e_end = s + len;
        if all_day {
            // Shown as the last day, not the day after it.
            e_end -= Days::days(1);
            e_end = e_end.max(s);
        }
        self.set_when(s, e_end, all_day);
        let series = res
            .master
            .as_ref()
            .is_some_and(|m| !m.event.rrule.is_empty() || !m.rdates.is_empty());
        g.set_ed_series(series && !occ_key.is_empty());
        let master_start = res.master.as_ref().map(|m| local_wall(&m.event.start));
        let rule = res
            .master
            .as_ref()
            .map(|m| m.event.rrule.clone())
            .unwrap_or_default();
        let sd = master_start.unwrap_or(s).date();
        self.fill_repeats(sd);
        match edit::repeat_of(&rule, sd) {
            Some(r) => g.set_ed_repeat(r as i32),
            None => g.set_ed_rule(rule.as_str().into()),
        }
        let alarm = item.alarms.first().map(|a| a.before_min);
        let alarm_index = REMINDERS.iter().position(|r| *r == alarm).unwrap_or(0) as i32;
        g.set_ed_reminder(alarm_index);
        g.set_ed_title(ev.summary.as_str().into());
        g.set_ed_location(ev.location.as_str().into());
        g.set_ed_notes(ev.description.as_str().into());
        g.set_ed_calendar(
            cals.iter()
                .position(|c| c.id == info.calendar_id)
                .unwrap_or(0) as i32,
        );
        if read_only {
            let who = ev
                .organizer
                .as_ref()
                .map(|o| {
                    if o.name.is_empty() {
                        o.addr.clone()
                    } else {
                        o.name.clone()
                    }
                })
                .unwrap_or_default();
            g.set_ed_organizer(who.into());
        }
        g.set_ed_read_only(read_only);
        let me = ev
            .organizer
            .as_ref()
            .filter(|_| info.mine)
            .map(|o| o.addr.clone())
            .unwrap_or_default();
        ed(|e| {
            e.id = Some(event);
            e.key = occ_key;
            e.occ = Some(s);
            e.series = master_start;
            e.ics = info.ics.clone();
            e.ours_alarm = info.ics.contains("X-KOMOREBI:1");
            e.alarm_index = alarm_index;
            e.me = me;
        });
        g.set_ed_new(false);
        g.set_editing(true);
    }

    /// The form as typed; an error sentence when it can't be read.
    fn read_form(&self) -> Result<Form, String> {
        let ui = self.ui();
        let g = ui.global::<Cal>();
        let df = day_first(self.lang);
        let bad_date = || t!(invoke_cal_ed_bad_date, t!(invoke_cal_date_hint, df).into());
        let sd = edit::parse_date(&g.get_ed_start_date(), df).ok_or_else(bad_date)?;
        let ed_ = edit::parse_date(&g.get_ed_end_date(), df).ok_or_else(bad_date)?;
        let all_day = g.get_ed_all_day();
        let (st, et) = if all_day {
            (chrono::NaiveTime::MIN, chrono::NaiveTime::MIN)
        } else {
            (
                edit::parse_time(&g.get_ed_start_time())
                    .ok_or_else(|| t!(invoke_cal_ed_bad_time))?,
                edit::parse_time(&g.get_ed_end_time()).ok_or_else(|| t!(invoke_cal_ed_bad_time))?,
            )
        };
        let index = g.get_ed_reminder();
        let mut alarm = REMINDERS
            .get(usize::try_from(index).unwrap_or(0))
            .copied()
            .flatten();
        // Another app's alarm left unchanged stays as it is (not doubled).
        if ed(|e| e.id.is_some() && !e.ours_alarm && e.alarm_index == index) {
            alarm = None;
        }
        Ok(Form {
            summary: g.get_ed_title().trim().to_string(),
            location: g.get_ed_location().trim().to_string(),
            description: g.get_ed_notes().to_string(),
            all_day,
            start: sd.and_time(st),
            end: ed_.and_time(et),
            rrule: String::new(),
            alarm_min: alarm,
        })
    }

    fn calendar_save(&mut self) {
        let mut form = match self.read_form() {
            Ok(f) => f,
            Err(e) => return self.ui().global::<Cal>().set_ed_error(e.into()),
        };
        let ui = self.ui();
        let g = ui.global::<Cal>();
        let (id, key, occ, series, cals, ics, me) = ed(|e| {
            (
                e.id,
                e.key.clone(),
                e.occ,
                e.series,
                e.cals.clone(),
                e.ics.clone(),
                e.me.clone(),
            )
        });
        let Some(&cal) = cals.get(usize::try_from(g.get_ed_calendar()).unwrap_or(0)) else {
            return g.set_ed_error(t!(invoke_cal_ed_read_only).into());
        };
        let only_this = g.get_ed_series() && g.get_ed_scope() == 0;
        if !only_this {
            let kept = g.get_ed_rule();
            if !kept.is_empty() {
                form.rrule = kept.to_string();
            } else {
                // The whole series moves by as much as this occurrence did.
                if let (Some(o), Some(s)) = (occ, series)
                    && id.is_some()
                {
                    let shift = form.start - o;
                    let len = form.end - form.start;
                    form.start = s + shift;
                    form.end = form.start + len;
                }
                form.rrule = edit::rrule(Repeat::from_code(g.get_ed_repeat()), form.start.date());
            }
        }
        let uid = match id {
            Some(_) => ical::parse_resource(ics.as_bytes())
                .map(|r| r.uid().to_string())
                .unwrap_or_default(),
            None => komorebi::mail::smtp::message_id("komorebi@local")
                .trim_matches(['<', '>'])
                .to_string(),
        };
        let now = chrono::Utc::now();
        let e = match edit::edit(&form, &uid, now, &komorebi::cal::local_utc) {
            Ok(e) => e,
            Err(FormError::EndBeforeStart) => {
                return g.set_ed_error(t!(invoke_cal_ed_end_before).into());
            }
            Err(FormError::BadTime) => return g.set_ed_error(t!(invoke_cal_ed_gap).into()),
        };
        let ts = now.timestamp();
        let old_cal = id
            .and_then(|i| self.db.event_info(i).ok().flatten())
            .map(|i| i.calendar_id);
        let done = match id {
            None => self
                .db
                .write_event(cal, None, &ical::write_event(&e), &me, ts)
                .map(drop),
            Some(i) if only_this => self.db.edit_occurrence(i, &key, &e, &me, ts),
            Some(i) => match ical::patch(&ics, &e) {
                Some(patched) if old_cal == Some(cal) => self
                    .db
                    .write_event(cal, Some(i), &patched, &me, ts)
                    .map(drop),
                // Another calendar: removed there, added here.
                Some(patched) => self
                    .db
                    .delete_event(i, false, ts)
                    .and_then(|()| self.db.write_event(cal, None, &patched, &me, ts))
                    .map(drop),
                None => Err(anyhow::anyhow!("unreadable event")),
            },
        };
        if let Err(err) = done {
            return g.set_ed_error(t!(invoke_cal_ed_failed, err.to_string().into()).into());
        }
        self.db.set_setting("cal_default", &cal.to_string()).ok();
        super::calendar::set_anchor(form.start.date());
        self.calendar_changed(&[Some(cal), old_cal]);
    }

    fn calendar_delete(&mut self) {
        let ui = self.ui();
        let g = ui.global::<Cal>();
        let (id, key) = ed(|e| (e.id, e.key.clone()));
        let Some(id) = id else {
            return;
        };
        let cal = self.db.event_info(id).ok().flatten().map(|i| i.calendar_id);
        let ts = chrono::Utc::now().timestamp();
        let done = if g.get_ed_read_only() {
            self.db.delete_event(id, true, ts)
        } else if g.get_ed_series() && g.get_ed_scope() == 0 {
            self.db.delete_occurrence(id, &key, ts)
        } else {
            self.db.delete_event(id, false, ts)
        };
        if let Err(err) = done {
            return g.set_ed_error(t!(invoke_cal_ed_failed, err.to_string().into()).into());
        }
        self.calendar_changed(&[cal]);
    }

    /// After a change: the editor closes, the view shows it, and a pass
    /// sends it for each CalDAV calendar touched.
    fn calendar_changed(&mut self, cals: &[Option<i64>]) {
        let ui = self.ui();
        let g = ui.global::<Cal>();
        g.set_editing(false);
        g.set_picker(0);
        let accounts: Vec<i64> = self
            .db
            .calendars()
            .unwrap_or_default()
            .into_iter()
            .filter(|c| cals.contains(&Some(c.id)))
            .filter_map(|c| c.account_id)
            .collect();
        self.calendar_reload();
        for a in accounts {
            self.start_calendar_sync(a);
        }
    }

    pub(super) fn calendar_resolve(&mut self, again: bool) {
        let ts = chrono::Utc::now().timestamp();
        let cals = self.db.resolve_conflicts(again, "", ts).unwrap_or_default();
        let cals: Vec<Option<i64>> = cals.into_iter().map(Some).collect();
        self.calendar_changed(&cals);
    }

    // ---- The date picker -----------------------------------------------

    fn picker_show(&mut self) {
        let Some(month) = ed(|e| e.picker) else {
            return;
        };
        let selected = ed(|e| e.picker_start).unwrap_or(month);
        let (cells, start) = self.month_cells(month, selected, false);
        PICKER_GRID.set(Some(start));
        let ui = self.ui();
        let g = ui.global::<Cal>();
        g.set_picker_title(format::month_title(self.lang, &month).into());
        g.set_picker_cells(ModelRc::new(VecModel::from(cells)));
    }

    fn picker_open(&mut self, which: i32) {
        let ui = self.ui();
        let g = ui.global::<Cal>();
        g.set_picker(which);
        if which == 0 {
            return;
        }
        let df = day_first(self.lang);
        let text = if which == 1 {
            g.get_ed_start_date()
        } else {
            g.get_ed_end_date()
        };
        let d = edit::parse_date(&text, df).unwrap_or_else(anchor);
        ed(|e| {
            e.picker = Some(d);
            e.picker_start = Some(d);
        });
        self.picker_show();
    }

    fn picker_step(&mut self, dir: i32) {
        ed(|e| {
            e.picker = e.picker.and_then(|m| {
                if dir > 0 {
                    m.checked_add_months(Months::new(1))
                } else {
                    m.checked_sub_months(Months::new(1))
                }
            });
        });
        self.picker_show();
    }

    fn picker_pick(&mut self, index: i32) {
        let Some(start) = PICKER_GRID.get() else {
            return;
        };
        let d = start + Days::days(i64::from(index.clamp(0, 41)));
        let ui = self.ui();
        let g = ui.global::<Cal>();
        let text: SharedString = edit::show_date(d, day_first(self.lang)).into();
        if g.get_picker() == 1 {
            // The end keeps its distance from the start.
            let df = day_first(self.lang);
            if let (Some(old), Some(end)) = (
                edit::parse_date(&g.get_ed_start_date(), df),
                edit::parse_date(&g.get_ed_end_date(), df),
            ) {
                let moved = end + (d - old);
                g.set_ed_end_date(edit::show_date(moved, df).into());
            }
            g.set_ed_start_date(text);
            self.fill_repeats(d);
        } else {
            g.set_ed_end_date(text);
        }
        g.set_picker(0);
    }

    /// Snapshots (`calendar:edit`): the sample's weekly series in the
    /// editor (else a new event).
    pub(super) fn calendar_sample_edit(&mut self) {
        let event = self.db.event_by_uid("s4").ok().flatten().map(|(id, _)| id);
        let key = KEYS.with_borrow(|k| k.iter().position(|(e, _)| Some(*e) == event));
        match key {
            Some(k) => self.calendar_open(k as i32),
            None => self.calendar_new_event(),
        }
    }
}

thread_local! {
    /// The picker grid's first day.
    static PICKER_GRID: Cell<Option<NaiveDate>> = const { Cell::new(None) };
}

/// `text` (a time) moved by `minutes`, as 24-hour `HH:MM`; unchanged
/// when it can't be read.
fn step_time(text: &str, minutes: i64) -> String {
    match edit::parse_time(text) {
        Some(t) => {
            // To the next quarter hour first, then by quarters.
            let (shifted, _) = t.overflowing_add_signed(Days::minutes(minutes));
            let m = chrono::Timelike::minute(&shifted);
            let snapped = shifted - Days::minutes(i64::from(m % 15));
            snapped.format("%H:%M").to_string()
        }
        None => text.to_string(),
    }
}

pub(super) fn wire(ui: &AppWindow) {
    let g = ui.global::<Cal>();
    g.on_new_event(|| {
        with_app(|a| a.calendar_new_event());
    });
    g.on_open(|k| {
        with_app(|a| a.calendar_open(k));
    });
    g.on_ed_save(|| {
        with_app(|a| a.calendar_save());
    });
    g.on_ed_delete(|| {
        with_app(|a| a.calendar_delete());
    });
    g.on_ed_cancel(|| {
        with_app(|a| {
            let ui = a.ui();
            let g = ui.global::<Cal>();
            g.set_editing(false);
            g.set_picker(0);
        });
    });
    g.on_ed_date_changed(|| {
        with_app(|a| {
            let ui = a.ui();
            let g = ui.global::<Cal>();
            if let Some(d) = edit::parse_date(&g.get_ed_start_date(), day_first(a.lang)) {
                a.fill_repeats(d);
            }
        });
    });
    g.on_step_time(|text, m| step_time(&text, i64::from(m)).into());
    g.on_picker_open(|w| {
        with_app(|a| a.picker_open(w));
    });
    g.on_picker_step(|d| {
        with_app(|a| a.picker_step(d));
    });
    g.on_picker_pick(|i| {
        with_app(|a| a.picker_pick(i));
    });
}

#[cfg(test)]
mod tests {
    use super::step_time;

    #[test]
    fn times_step_by_quarters() {
        assert_eq!(step_time("09:00", 15), "09:15");
        assert_eq!(step_time("09:07", 15), "09:15");
        assert_eq!(step_time("09:07", -15), "08:45");
        assert_eq!(step_time("23:50", 15), "00:00");
        assert_eq!(step_time("later", 15), "later");
    }
}
