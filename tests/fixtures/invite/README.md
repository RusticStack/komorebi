# Meeting invitation fixtures

Eight synthetic messages for `tests/invite_corpus.rs` and snapshot QA
(`seed -- <new.sqlite3> 0 tests/fixtures/invite`; `KOMOREBI_SNAPSHOT_SELECT=n`
opens file n+1, and `KOMOREBI_SNAPSHOT_ACTION=rsvp` with `SELECT=0` shows the
reply prompt). **None of it is real mail**: addresses use `example.*` and
`komorebi.invalid` domains, the invitee is the seed account
(`seed@komorebi.invalid`), and the calendar parts are written by hand in the
shapes Google Calendar and Exchange send. Events are in 2027 (or recur until
then), so they are not yet past. The oversized case (over 256 KB) is built in
the test instead of committed.

| File | What it models | Expected card |
|---|---|---|
| `01-google-request` | Google: `text/calendar` alternative plus the same event as `invite.ics`, UTC times, `X-GOOGLE-CONFERENCE`, a VALARM | Accept/Tentative/Decline, Join, Details; only `invite.ics` listed |
| `02-outlook-request` | Exchange: quoted-printable calendar, Windows TZID `W. Europe Standard Time` with its VTIMEZONE, the Teams URL property | converted with the VTIMEZONE (UTC+1 on 24 March) |
| `03-cancel` | `METHOD:CANCEL`, `STATUS:CANCELLED`, SEQUENCE 1 | Cancelled, struck title, no answers |
| `04-update` | REQUEST with SEQUENCE 2, a meeting link in LOCATION | Updated invitation, answers, Join |
| `05-publish-ics-file` | `METHOD:PUBLISH` as an attached `.ics` file, DURATION, URL | no answers (nothing to answer), Join |
| `06-all-day` | `VALUE=DATE` start and (exclusive) end | Sat - Sun, all day |
| `07-recurring-weekly` | RRULE weekly on Mon and Wed until a date, TZID `Europe/Lisbon` with no VTIMEZONE | "Repeats weekly on Mon, Wed until 2027-12-31", converted with Windows' rules for the zone (UTC+1 on 12 October); a zone Windows doesn't know keeps its wall time, labelled with the zone |
| `08-malformed` | a calendar part whose DTSTART is not a date | no card: an ordinary message |

Keep CRLF line endings when editing them (`.gitattributes`).
