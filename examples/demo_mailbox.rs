//! A fictional demo mailbox for screenshots (README, website): one account,
//! a few folders, about twenty inbox messages with unread and flagged
//! states, an attachment and a formatted HTML newsletter, a few contacts,
//! rules and a signature. Every name, address and domain is invented
//! (`example.com`, `example.org`), nothing touches the network.
//!
//! ```text
//! cargo run --release --example demo_mailbox -- <new.sqlite3>
//! $env:KOMOREBI_OFFLINE=1; $env:KOMOREBI_DB="<new.sqlite3>"; $env:KOMOREBI_SNAPSHOT="shot.png"; komorebi.exe
//! ```
//!
//! Message dates are relative to the moment the cache is made, so the list
//! reads "today", "yesterday" and the days before. The newsletter is the
//! third row (`KOMOREBI_SNAPSHOT_SELECT=2`). It refuses the app's own cache
//! directory and any existing non-empty file.

use anyhow::{Context, Result, bail};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use komorebi::contacts::{Contact, Email, Phone};
use komorebi::db::{Db, DraftRow, NewAccount, NewHeader};
use komorebi::mail::{mime, rich::Doc};
use komorebi::providers::{Security, Server};
use komorebi::role::Role;
use komorebi::rules::{Condition, Field, Op, Rule};
use komorebi::seed;
use std::path::PathBuf;

const ME_NAME: &str = "Avery Stone";
const ME: &str = "avery.stone@example.com";

/// One message to cache.
struct Msg {
    /// Minutes before the cache was made.
    ago: i64,
    from: (&'static str, &'static str),
    subject: &'static str,
    body: &'static str,
    html: Option<String>,
    attach: Option<(&'static str, &'static str)>,
    seen: bool,
    flagged: bool,
    answered: bool,
    unsubscribe: Option<&'static str>,
}

fn m(
    ago: i64,
    from: (&'static str, &'static str),
    subject: &'static str,
    body: &'static str,
) -> Msg {
    Msg {
        ago,
        from,
        subject,
        body,
        html: None,
        attach: None,
        seen: true,
        flagged: false,
        answered: false,
        unsubscribe: None,
    }
}

impl Msg {
    fn unread(mut self) -> Self {
        self.seen = false;
        self
    }
    fn flag(mut self) -> Self {
        self.flagged = true;
        self
    }
    fn replied(mut self) -> Self {
        self.answered = true;
        self
    }
    fn file(mut self, name: &'static str, mime: &'static str) -> Self {
        self.attach = Some((name, mime));
        self
    }
}

/// A header value that may hold non-ASCII text (RFC 2047).
fn word(s: &str) -> String {
    if s.is_ascii() {
        s.to_string()
    } else {
        format!("=?UTF-8?B?{}?=", B64.encode(s))
    }
}

fn wrap76(s: &str) -> String {
    s.as_bytes()
        .chunks(76)
        .map(|c| String::from_utf8_lossy(c).into_owned())
        .collect::<Vec<_>>()
        .join("\r\n")
}

fn crlf(s: &str) -> String {
    s.replace("\r\n", "\n").replace('\n', "\r\n")
}

fn eml(msg: &Msg, now: i64, uid: u32) -> Vec<u8> {
    let date = chrono::DateTime::from_timestamp(now - msg.ago * 60, 0)
        .expect("date")
        .to_rfc2822();
    let mut h = String::new();
    h += &format!("Message-ID: <demo-{uid}@example.com>\r\n");
    h += &format!("Date: {date}\r\n");
    h += &format!("From: {} <{}>\r\n", word(msg.from.0), msg.from.1);
    h += &format!("To: {ME_NAME} <{ME}>\r\n");
    h += &format!("Subject: {}\r\n", word(msg.subject));
    h += "MIME-Version: 1.0\r\n";
    if let Some(u) = msg.unsubscribe {
        h += &format!("List-Unsubscribe: <{u}>\r\n");
    }
    let text = crlf(msg.body);
    let alt = |boundary: &str| -> String {
        let mut s =
            format!("--{boundary}\r\nContent-Type: text/plain; charset=utf-8\r\n\r\n{text}\r\n");
        if let Some(html) = &msg.html {
            s += &format!(
                "--{boundary}\r\nContent-Type: text/html; charset=utf-8\r\n\r\n{}\r\n",
                crlf(html)
            );
        }
        s += &format!("--{boundary}--\r\n");
        s
    };
    let body = match (&msg.html, msg.attach) {
        (None, None) => {
            h += "Content-Type: text/plain; charset=utf-8\r\n\r\n";
            text
        }
        (Some(_), None) => {
            h += "Content-Type: multipart/alternative; boundary=\"alt\"\r\n\r\n";
            alt("alt")
        }
        (_, Some((name, kind))) => {
            h += "Content-Type: multipart/mixed; boundary=\"mix\"\r\n\r\n";
            let inner = if msg.html.is_some() {
                format!(
                    "--mix\r\nContent-Type: multipart/alternative; boundary=\"alt\"\r\n\r\n{}",
                    alt("alt")
                )
            } else {
                format!("--mix\r\nContent-Type: text/plain; charset=utf-8\r\n\r\n{text}\r\n")
            };
            // A small stand-in file: the reader lists it, nothing opens it.
            let payload = format!("%PDF-1.4\n% fictional demo attachment: {name}\n").repeat(40);
            format!(
                "{inner}--mix\r\nContent-Type: {kind}; name=\"{name}\"\r\n\
                 Content-Disposition: attachment; filename=\"{name}\"\r\n\
                 Content-Transfer-Encoding: base64\r\n\r\n{}\r\n--mix--\r\n",
                wrap76(&B64.encode(payload))
            )
        }
    };
    (h + &body).into_bytes()
}

fn store(
    db: &mut Db,
    (account, folder): (i64, i64),
    msgs: &[Msg],
    now: i64,
    uid0: u32,
) -> Result<()> {
    for (i, msg) in msgs.iter().enumerate() {
        let uid = uid0 + i as u32;
        let raw = eml(msg, now, uid);
        let body = mime::parse_body(&raw);
        let header = NewHeader {
            uid,
            date: now - msg.ago * 60,
            size: raw.len() as i64,
            seen: msg.seen,
            flagged: msg.flagged,
            answered: msg.answered,
            has_attach: !body.attachments.is_empty(),
            ..mime::parse_header(&raw)
        };
        db.insert_headers(account, folder, std::slice::from_ref(&header))?;
        let id = db
            .message_ids(&[folder], "")?
            .into_iter()
            .find(|&id| {
                db.message(id)
                    .is_ok_and(|x| x.is_some_and(|x| x.uid == uid))
            })
            .context("added message vanished")?;
        db.set_body(
            id,
            &raw,
            &body.preview(),
            header.has_attach,
            Default::default(),
        )?;
    }
    Ok(())
}

fn newsletter_html() -> String {
    let item = |title: &str, text: &str| {
        format!(
            "<td valign=\"top\" width=\"50%\" style=\"padding:0 10px 0 0;\">\
             <p style=\"margin:0 0 4px;font:bold 15px Georgia,serif;color:#2f4a36;\">{title}</p>\
             <p style=\"margin:0 0 16px;font:14px/1.55 Georgia,serif;color:#444;\">{text}</p></td>"
        )
    };
    format!(
        r##"<!doctype html><html><body style="margin:0;padding:0;background:#eef2ea;">
<table role="presentation" width="100%" cellpadding="0" cellspacing="0" style="background:#eef2ea;"><tr><td align="center" style="padding:24px 12px;">
<table role="presentation" width="600" cellpadding="0" cellspacing="0" style="background:#ffffff;border-radius:6px;">
<tr><td bgcolor="#2f4a36" style="padding:28px 36px;border-radius:6px 6px 0 0;">
<p style="margin:0;font:12px Arial,sans-serif;letter-spacing:3px;color:#b9d2b6;">NO. 42 &middot; NOVEMBER</p>
<p style="margin:6px 0 0;font:bold 30px Georgia,serif;color:#ffffff;">The Slow Garden</p>
<p style="margin:4px 0 0;font:italic 15px Georgia,serif;color:#d6e6d3;">Notes for people who garden without hurrying</p></td></tr>
<tr><td style="padding:30px 36px 8px;">
<h1 style="margin:0 0 12px;font:bold 24px/1.25 Georgia,serif;color:#1f2d23;">Planting for the quiet months</h1>
<p style="margin:0 0 14px;font:16px/1.6 Georgia,serif;color:#333;">The first hard frost came early this year, and the beds went quiet almost overnight. That is not the end of the season. It is the start of the part where the work is small, the light is low, and the garden pays you back for last spring.</p>
<p style="margin:0 0 14px;font:16px/1.6 Georgia,serif;color:#333;">This issue is about what to do now, and what to leave alone.</p></td></tr>
<tr><td style="padding:6px 36px 14px;">
<table role="presentation" width="100%" cellpadding="0" cellspacing="0"><tr><td style="border-left:4px solid #8fb58a;padding:6px 0 6px 16px;font:italic 18px/1.5 Georgia,serif;color:#2f4a36;">&ldquo;A bulb is a promise you make to yourself in the dark, and keep in the spring.&rdquo;</td></tr></table></td></tr>
<tr><td style="padding:10px 36px 0;"><h2 style="margin:0 0 12px;font:bold 18px Georgia,serif;color:#1f2d23;border-bottom:1px solid #dfe7db;padding-bottom:8px;">This week in the garden</h2>
<table role="presentation" width="100%" cellpadding="0" cellspacing="0"><tr>{}{}</tr><tr>{}{}</tr></table></td></tr>
<tr><td style="padding:6px 36px 0;"><h2 style="margin:0 0 10px;font:bold 18px Georgia,serif;color:#1f2d23;">Three jobs for a cold Saturday</h2>
<ol style="margin:0 0 18px;padding-left:22px;font:15px/1.7 Georgia,serif;color:#333;"><li>Plant garlic, pointed end up, about five centimetres deep.</li><li>Cut back the perennials that have gone to mush, and leave the seed heads for the finches.</li><li>Move the compost heap a metre to the left. It will thank you by March.</li></ol></td></tr>
<tr><td align="center" style="padding:6px 36px 30px;"><a href="https://slowgarden.example.org/issues/42" style="display:inline-block;background:#c8553d;color:#ffffff;text-decoration:none;font:bold 15px Arial,sans-serif;padding:13px 28px;border-radius:4px;">Read the full guide</a></td></tr>
<tr><td bgcolor="#f5f7f2" style="padding:20px 36px;border-radius:0 0 6px 6px;font:12px/1.6 Arial,sans-serif;color:#6b7769;">You are receiving this because you signed up at slowgarden.example.org. <a href="https://slowgarden.example.org/unsubscribe" style="color:#6b7769;">Unsubscribe</a> &middot; Slow Garden Press, 12 Orchard Lane, Exampleton</td></tr>
</table></td></tr></table></body></html>"##,
        item(
            "Garlic and shallots",
            "Soil is still warm enough to root before the freeze. Plant now, mulch lightly."
        ),
        item(
            "Tulips, finally",
            "Wait for the first cold week, then go deep: fifteen centimetres keeps the squirrels guessing."
        ),
        item(
            "Houseplants",
            "Water less than you think you should, and turn the pots a quarter every Sunday."
        ),
        item(
            "The compost",
            "Cover it. Winter rain turns a good heap into soup."
        ),
    )
}

fn inbox() -> Vec<Msg> {
    let pr = ("Priya Raman", "priya.raman@example.org");
    let mo = ("Marcus Oyelaran", "marcus.oyelaran@brightloom.example.com");
    let ev = ("Elena Vasquez", "elena.vasquez@example.net");
    let tf = ("Tomás Ferreira", "tomas.ferreira@example.com");
    let hb = ("Hannah Brooks", "hannah.brooks@example.org");
    let jw = ("Jun Watanabe", "jun.watanabe@brightloom.example.com");
    let sl = ("Sofia Lindqvist", "sofia.lindqvist@example.net");
    let dok = ("Daniel Okafor", "daniel.okafor@example.com");
    let mt = ("Mei Tanaka", "mei.tanaka@brightloom.example.com");
    let lm = ("Lucas Moreau", "lucas.moreau@example.org");
    let an = ("Amara Nwosu", "amara.nwosu@example.net");
    let og = ("Oliver Grant", "oliver.grant@example.com");
    let cd = ("Camille Dubois", "camille.dubois@example.org");
    let mut news = m(
        125,
        ("The Slow Garden", "hello@slowgarden.example.org"),
        "The Slow Garden No. 42: Planting for the quiet months",
        "The Slow Garden, No. 42\n\nPlanting for the quiet months\n\nThe first hard frost came early this year, and the beds went quiet almost overnight.\n\nRead the full guide: https://slowgarden.example.org/issues/42\n",
    )
    .unread();
    news.html = Some(newsletter_html());
    news.unsubscribe = Some("mailto:unsubscribe@slowgarden.example.org");
    vec![
        m(12, pr, "Re: Launch checklist for Thursday", "Hi Avery,\n\nI went through the checklist and everything on my side is ready. The only open item is the press page: Jun is waiting on the final screenshots before he can publish.\n\nCan you confirm the time for the sync tomorrow? I have 10:30 free.\n\nPriya").unread(),
        m(48, mo, "Draft agenda for the offsite", "Hi Avery,\n\nHere is the first draft of the agenda for the March offsite. Two things I would like your eyes on: the Tuesday afternoon block, and whether we need a second room.\n\nNo rush, but I would like to send it out on Friday.\n\nMarcus").unread().flag().file("offsite-agenda-draft.pdf", "application/pdf"),
        news,
        m(190, ev, "Photos from Saturday", "Hey!\n\nI finally sorted the photos from the lake. The best ones are in the shared album, the rest are blurry and will stay that way.\n\nThanks again for driving.\n\nElena").replied(),
        m(300, ("Harbor Street Books", "orders@harborstreetbooks.example.com"), "Your order HB-20418 has shipped", "Good news: your order HB-20418 is on its way.\n\n  1 x The Cartographer's Apprentice (paperback)\n  1 x A Field Guide to Quiet Places\n\nEstimated delivery: Thursday.\n\nHarbor Street Books").unread(),
        m(560, tf, "Lunch on Friday?", "Avery, are you free for lunch on Friday? There is a new place near the station that does a proper bowl of soup.\n\nTomás").flag(),
        m(1220, hb, "Invoice 2026-114 for October", "Hello Avery,\n\nAttached is the invoice for October. Payment terms are 30 days as usual. Let me know if anything looks off.\n\nBest,\nHannah Brooks").file("invoice-2026-114.pdf", "application/pdf"),
        m(1290, ("Riverside Community Choir", "secretary@riversidechoir.example.org"), "Rehearsal moved to 7:30 pm", "Dear members,\n\nThis week's rehearsal moves to 7:30 pm because the hall is hosting the farmers' market until 7. Please bring the new score.\n\nSee you there,\nThe Riverside Choir committee").unread(),
        m(1500, jw, "Notes from the design review", "Hi all,\n\nNotes from today's design review:\n\n- The new sidebar spacing is approved.\n- We keep the old icon set for one more release.\n- Mei will redo the empty states.\n\nNext review is in two weeks.\n\nJun"),
        m(2900, sl, "Quick question about the budget", "Hi Avery,\n\nDo we have room in the Q1 budget for two more licences? If yes, I will send the purchase request today.\n\nSofia").replied(),
        m(3100, dok, "Welcome aboard!", "Hi Avery,\n\nI am Daniel, I joined the support team this week. Priya said you are the person to ask about the release process, so consider this my first question: where do I find the checklist?\n\nThanks!\nDaniel"),
        m(4300, ("Lakeside City Library", "notices@lakesidelibrary.example.org"), "Two items are due on Monday", "Dear reader,\n\nThe following items are due on Monday:\n\n  A Field Guide to Quiet Places\n  Letters from a Slow Year\n\nYou can renew them online until Sunday.\n\nLakeside City Library"),
        m(4400, mt, "Updated slides for Wednesday", "Avery,\n\nHere are the updated slides. I added the timeline you asked for on slide 7 and trimmed the appendix.\n\nMei").file("wednesday-review.pdf", "application/pdf"),
        m(5800, lm, "Re: Bike ride Sunday", "Count me in. I will bring the pump this time, since someone forgot it last month.\n\nMeet at the bridge at 9?\n\nLucas"),
        m(7300, ("Fernbank Energy", "billing@fernbank-energy.example.com"), "Your October statement is ready", "Your statement for October is ready to view in your account.\n\nAmount due: 62.40\nDue date: 2 December\n\nThank you,\nFernbank Energy"),
        m(8600, an, "Congratulations!", "Avery, I just heard the news. Congratulations on the new role, it is well deserved. Dinner is on me when things calm down.\n\nAmara"),
        m(8700, og, "Can you review my pull request?", "Hi Avery,\n\nWhen you have a moment, could you look at my pull request for the search fix? It is small, about forty lines.\n\nThanks,\nOliver").flag(),
        m(11300, cd, "The recipe you asked for", "Here is the recipe for the plum cake, as promised. The trick is to leave the plums in big pieces and to not open the oven for the first 30 minutes.\n\nCamille"),
        m(12900, ("Noah Fischer", "noah.fischer@example.net"), "Meeting notes, 6 November", "Hello,\n\nMy notes from our meeting on 6 November:\n\n1. Move the weekly sync to Thursdays.\n2. Share the roadmap draft by the 20th.\n3. Revisit the hiring plan in December.\n\nShout if I missed anything.\n\nNoah"),
        m(14400, ("Trailhead Cycling Club", "events@trailheadcycling.example.org"), "Winter rides start next week", "Hi riders,\n\nThe winter schedule starts next week: short loops on Saturday mornings, coffee after. Lights are mandatory from now on.\n\nThe Trailhead crew"),
    ]
}

fn main() -> Result<()> {
    let path = PathBuf::from(
        std::env::args()
            .nth(1)
            .context("usage: demo_mailbox <new.sqlite3>")?,
    );
    seed::refuse_real_cache(&path)?;
    if !path
        .to_string_lossy()
        .to_lowercase()
        .contains("komorebi-shots")
    {
        bail!("the demo cache belongs in a throwaway folder named komorebi-shots");
    }
    let mut db = Db::open(&path)?;
    let now = chrono::Utc::now().timestamp();
    let server = |host: &str, port| Server {
        host: host.into(),
        port,
        security: Security::Tls,
    };
    let account = db.add_account(&NewAccount {
        imap_user: String::new(),
        smtp_user: String::new(),
        email: ME.into(),
        name: ME_NAME.into(),
        imap: server("imap.example.com", 993),
        smtp: server("smtp.example.com", 465),
        save_sent: true,
        smtp_no_auth: false,
    })?;
    let folder = |name: &str, role| db.upsert_folder(account, name, name, 0, role);
    let inbox_id = db.upsert_folder(account, "INBOX", "Inbox", 0, Role::Inbox)?;
    let drafts = folder("Drafts", Role::Drafts)?;
    let sent = folder("Sent", Role::Sent)?;
    let archive = folder("Archive", Role::Archive)?;
    let junk = folder("Junk", Role::Junk)?;
    let trash = folder("Trash", Role::Trash)?;
    let receipts = folder("Receipts", Role::Other)?;
    let projects = folder("Projects", Role::Other)?;
    let newsletters = folder("Newsletters", Role::Other)?;
    let _ = drafts;

    let a = account;
    store(&mut db, (a, inbox_id), &inbox(), now, 1)?;

    let me = (ME_NAME, ME);
    store(
        &mut db,
        (a, sent),
        &[
            m(
                95,
                me,
                "Re: Lunch on Friday?",
                "Friday works. Twelve-thirty?\n\nAvery",
            ),
            m(
                1700,
                me,
                "Reply to Sofia",
                "Yes, there is room. Go ahead and send the request.\n\nAvery",
            ),
            m(
                4000,
                me,
                "Offsite venues",
                "Marcus, here are the three venues I liked. The second one has a proper kitchen.\n\nAvery",
            ),
        ],
        now,
        1,
    )?;
    store(
        &mut db,
        (a, archive),
        &[
            m(
                26000,
                ("Hannah Brooks", "hannah.brooks@example.org"),
                "Invoice 2026-101 for September",
                "Attached is the invoice for September.\n\nHannah",
            )
            .file("invoice-2026-101.pdf", "application/pdf"),
            m(
                30000,
                ("Jun Watanabe", "jun.watanabe@brightloom.example.com"),
                "Icon set decision",
                "We keep the current icon set until the next major release.\n\nJun",
            ),
            m(
                41000,
                ("Priya Raman", "priya.raman@example.org"),
                "Onboarding plan",
                "Here is the onboarding plan for the new hires.\n\nPriya",
            ),
        ],
        now,
        1,
    )?;
    store(
        &mut db,
        (a, receipts),
        &[
            m(
                2000,
                (
                    "Harbor Street Books",
                    "orders@harborstreetbooks.example.com",
                ),
                "Receipt for order HB-20377",
                "Thank you for your order. Total: 31.50\n\nHarbor Street Books",
            )
            .unread(),
            m(
                9000,
                ("Fernbank Energy", "billing@fernbank-energy.example.com"),
                "Your September statement",
                "Your September statement is ready.\n\nFernbank Energy",
            ),
            m(
                21000,
                ("Lakeside Cycles", "service@lakesidecycles.example.com"),
                "Service receipt",
                "Thank you for visiting. Service total: 48.00\n\nLakeside Cycles",
            ),
        ],
        now,
        1,
    )?;
    store(
        &mut db,
        (a, projects),
        &[
            m(
                700,
                ("Marcus Oyelaran", "marcus.oyelaran@brightloom.example.com"),
                "Offsite budget",
                "Rough numbers for the offsite are in the sheet.\n\nMarcus",
            )
            .unread(),
            m(
                6000,
                ("Mei Tanaka", "mei.tanaka@brightloom.example.com"),
                "Empty states, round two",
                "New empty states are ready for review.\n\nMei",
            ),
        ],
        now,
        1,
    )?;
    store(
        &mut db,
        (a, newsletters),
        &[
            m(
                1500,
                ("The Slow Garden", "hello@slowgarden.example.org"),
                "The Slow Garden No. 41: Leaves, leaves, leaves",
                "No. 41: what to do with all those leaves.\n",
            )
            .unread(),
            m(
                11000,
                (
                    "Trailhead Cycling Club",
                    "events@trailheadcycling.example.org",
                ),
                "Autumn newsletter",
                "Autumn at the club: results and photos.\n",
            ),
            m(
                19000,
                ("The Slow Garden", "hello@slowgarden.example.org"),
                "The Slow Garden No. 40: The last tomatoes",
                "No. 40: the last tomatoes of the year.\n",
            ),
        ],
        now,
        1,
    )?;
    store(
        &mut db,
        (a, junk),
        &[m(3200, ("Prize Desk", "winner@prizes.example.net"), "You have been selected", "Congratulations, you have been selected for a prize. This is an obviously fictional message.\n").unread()],
        now,
        1,
    )?;
    store(
        &mut db,
        (a, trash),
        &[m(
            5000,
            ("Old Subscription", "noreply@oldsubscription.example.org"),
            "Your trial has ended",
            "Your trial has ended.\n",
        )],
        now,
        1,
    )?;

    // Two saved drafts: a reply and a new message.
    db.insert_draft(&DraftRow {
        account_id: account,
        to: "Marcus Oyelaran <marcus.oyelaran@brightloom.example.com>".into(),
        subject: "Re: Draft agenda for the offsite".into(),
        body: "Hi Marcus,\n\nThe Tuesday block looks right to me. One room is enough if we split the breakouts.\n".into(),
        message_id: "<demo-draft-1@example.com>".into(),
        ..Default::default()
    })?;

    // A signature, the account's default.
    let sig = db.add_signature(
        "Avery",
        &Doc::from_plain("Avery Stone\nProduct lead, Example Studio"),
        None,
    )?;
    db.set_account_signatures(account, Some(sig), Some(sig))?;

    // Rules.
    let cond = |field, op, value: &str| Condition {
        field,
        op,
        value: value.into(),
    };
    for rule in [
        Rule {
            name: "Receipts and invoices".into(),
            enabled: true,
            match_any: true,
            move_to: Some("Receipts".into()),
            conditions: vec![
                cond(Field::From, Op::Contains, "billing@"),
                cond(Field::Subject, Op::Contains, "receipt"),
                cond(Field::Subject, Op::StartsWith, "Invoice"),
            ],
            ..Rule::default()
        },
        Rule {
            name: "Newsletters".into(),
            enabled: true,
            match_any: true,
            stop: true,
            move_to: Some("Newsletters".into()),
            mark_read: false,
            conditions: vec![
                cond(Field::From, Op::EndsWith, "slowgarden.example.org"),
                cond(Field::From, Op::EndsWith, "trailheadcycling.example.org"),
            ],
            ..Rule::default()
        },
        Rule {
            name: "Flag the design team".into(),
            enabled: true,
            flag: true,
            conditions: vec![cond(Field::From, Op::Contains, "brightloom.example.com")],
            ..Rule::default()
        },
        Rule {
            name: "Offsite planning".into(),
            enabled: true,
            move_to: Some("Projects".into()),
            conditions: vec![cond(Field::Subject, Op::Contains, "offsite")],
            ..Rule::default()
        },
        Rule {
            name: "Junk prizes".into(),
            enabled: false,
            trash: true,
            conditions: vec![cond(Field::Subject, Op::Contains, "selected")],
            ..Rule::default()
        },
    ] {
        db.save_rule(account, &rule)?;
    }

    // Contacts, in the local address book.
    let book = db.local_book()?;
    let people: &[(&str, &str, &str, &str, &str, &str, &[&str])] = &[
        (
            "Amara",
            "Nwosu",
            "amara.nwosu@example.net",
            "+1 555 0101",
            "Nwosu Studio",
            "",
            &["Friends"],
        ),
        (
            "Camille",
            "Dubois",
            "camille.dubois@example.org",
            "+1 555 0102",
            "",
            "",
            &["Friends"],
        ),
        (
            "Daniel",
            "Okafor",
            "daniel.okafor@example.com",
            "+1 555 0103",
            "Example Studio",
            "Support",
            &["Work"],
        ),
        (
            "Elena",
            "Vasquez",
            "elena.vasquez@example.net",
            "+1 555 0104",
            "",
            "",
            &["Friends"],
        ),
        (
            "Hannah",
            "Brooks",
            "hannah.brooks@example.org",
            "+1 555 0105",
            "Brooks & Co. Accounting",
            "Accountant",
            &["Work"],
        ),
        (
            "Jun",
            "Watanabe",
            "jun.watanabe@brightloom.example.com",
            "+1 555 0106",
            "Brightloom",
            "Web lead",
            &["Work"],
        ),
        (
            "Lucas",
            "Moreau",
            "lucas.moreau@example.org",
            "+1 555 0107",
            "",
            "",
            &["Friends"],
        ),
        (
            "Marcus",
            "Oyelaran",
            "marcus.oyelaran@brightloom.example.com",
            "+1 555 0108",
            "Brightloom",
            "Operations",
            &["Work"],
        ),
        (
            "Mei",
            "Tanaka",
            "mei.tanaka@brightloom.example.com",
            "+1 555 0109",
            "Brightloom",
            "Designer",
            &["Work"],
        ),
        (
            "Oliver",
            "Grant",
            "oliver.grant@example.com",
            "+1 555 0110",
            "Example Studio",
            "Engineer",
            &["Work"],
        ),
        (
            "Priya",
            "Raman",
            "priya.raman@example.org",
            "+1 555 0111",
            "Example Studio",
            "Support lead",
            &["Work"],
        ),
        (
            "Sofia",
            "Lindqvist",
            "sofia.lindqvist@example.net",
            "+1 555 0112",
            "Example Studio",
            "Finance",
            &["Work"],
        ),
        (
            "Tomás",
            "Ferreira",
            "tomas.ferreira@example.com",
            "+1 555 0113",
            "",
            "",
            &["Friends"],
        ),
    ];
    for (given, family, email, phone, org, title, groups) in people {
        db.save_contact(&mut Contact {
            book_id: book,
            name: format!("{given} {family}"),
            given: (*given).into(),
            family: (*family).into(),
            org: (*org).into(),
            title: (*title).into(),
            emails: vec![Email {
                addr: (*email).into(),
                label: "work".into(),
                pref: true,
            }],
            phones: vec![Phone {
                number: (*phone).into(),
                label: "mobile".into(),
            }],
            groups: groups.iter().map(|g| (*g).into()).collect(),
            ..Contact::default()
        })?;
    }

    println!("demo mailbox written to {}", path.display());
    Ok(())
}
