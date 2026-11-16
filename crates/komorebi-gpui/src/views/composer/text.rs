//! The composer's sentences (docs/GPUI-I18N.md): the views keep typed
//! values (a `Kind`, a `SendBlock`, a `Guard`, an `AttachFail`) and call
//! these in `render`. Each is the same catalog entry as the Slint app's
//! (`ui/tr.slint` "Composing" and `ui/app.slint`'s composer), so it is
//! translated already.

use komorebi::compose::{AttachFail, AttachWhy, Field, Guard, Heads, Kind, SendBlock};
use komorebi::i18n::Lang;
use komorebi_i18n::tr;

/// The composer's title for a draft of `kind`.
pub fn title(kind: Kind) -> String {
    match kind {
        Kind::New => tr!("New message"),
        Kind::Reply => tr!("Reply"),
        Kind::ReplyAll => tr!("Reply all"),
        Kind::Forward => tr!("Forward"),
    }
}

/// The lines a reply's or forward's quote starts with, in the language
/// shown. The date is `komorebi::format::full_date`, as the Slint app's.
pub fn heads(lang: Lang) -> Heads {
    Heads {
        lang,
        wrote: |date, name| tr!("On {0}, {1} wrote:", date, name),
        when: komorebi::format::full_date,
        forwarded: tr!("---------- Forwarded message ----------"),
        from: tr!("forwarded header" => "From"),
        date: tr!("forwarded header" => "Date"),
        subject: tr!("forwarded header" => "Subject"),
        to: tr!("forwarded header" => "To"),
    }
}

/// Why Send did not send, under the toolbar.
pub fn send_block(b: SendBlock) -> String {
    match b {
        SendBlock::FixAddress(Field::To) => tr!("Fix the To address to send."),
        SendBlock::FixAddress(Field::Cc) => tr!("Fix the Cc address to send."),
        SendBlock::FixAddress(_) => tr!("Fix the Bcc address to send."),
        SendBlock::NoRecipient => tr!("Add a recipient to send."),
    }
}

/// What the send guards' bar asks.
pub fn guard(g: Guard) -> String {
    match g {
        Guard::Both => {
            tr!("No subject, and the text mentions an attachment but nothing is attached.")
        }
        Guard::NoSubject => tr!("This message has no subject."),
        Guard::NoAttachment => tr!("The text mentions an attachment, but nothing is attached."),
    }
}

/// An address field's inline error.
pub fn address_invalid(address: &str) -> String {
    tr!("“{0}” is not a valid address", address)
}

/// To's inline line after Send with no recipient.
pub fn add_recipient() -> String {
    tr!("Add a recipient to send.")
}

/// The header caption once the draft is saved.
pub fn draft_saved_at(time: &str) -> String {
    tr!("Draft saved {0}", time)
}

/// The status line after the composer closed with a draft kept.
pub fn draft_saved() -> String {
    tr!("Draft saved")
}

/// The attachments' total size (Komorebi sets no limit, so it is only
/// information).
pub fn attach_total(size: &str) -> String {
    tr!("Attachments: {0}", size)
}

/// One file that could not be attached.
pub fn attach_fail(f: &AttachFail) -> String {
    match &f.why {
        AttachWhy::Folder => tr!("{0}: folders can't be attached", f.name),
        AttachWhy::Io(why) => tr!("{0}: {1}", f.name, why),
    }
}

/// "(no subject)" in the undo-send offer.
pub fn no_subject() -> String {
    tr!("(no subject)")
}

/// A saved draft could not be opened again.
pub fn draft_gone() -> String {
    tr!("This draft is no longer saved.")
}

/// Saved attachments that could not be read back.
pub fn draft_left_out(names: &str) -> String {
    tr!("Couldn't load {0}; left out of this draft.", names)
}

/// Send with no account.
pub fn add_account_send() -> String {
    tr!("Add an account to send messages.")
}

/// A reply or forward of a message whose body is not here yet.
pub fn still_downloading() -> String {
    tr!("The message is still downloading.")
}

/// New mail with no account.
pub fn add_account_write() -> String {
    tr!("Add an account to write messages.")
}

/// Another draft took the composer; the one before stays in Drafts.
pub fn earlier_draft_saved() -> String {
    tr!("Earlier draft saved in Drafts.")
}

pub fn draft_kept_in_tab() -> String {
    tr!("Draft kept in a tab")
}

/// Discard asks first when something was written.
pub fn discard_question() -> (String, String, String, String) {
    (
        tr!("Discard this message?"),
        tr!("It is deleted here and from your Drafts folder on the server, with its attachments."),
        tr!("Discard"),
        tr!("Cancel"),
    )
}

/// A size as the attachments show it: `12 KB`, `1.5 MB` (pt-PT: a comma
/// and a no-break space), the library's `komorebi::format::size`.
pub fn size(lang: Lang, bytes: usize) -> String {
    komorebi::format::size(lang, bytes)
}

/// The time of day for "Draft saved 14:05" (`komorebi::format::time`, as
/// the Slint app).
pub fn time(ts: i64) -> String {
    komorebi::format::time(ts)
}

/// "Couldn't save the draft: {0}".
pub fn could_not_save(detail: &str) -> String {
    tr!("Couldn't save the draft: {0}", detail)
}

/// The undo offer while a send waits.
pub fn sending_in(subject: &str, secs: u64) -> String {
    tr!("Sending “{0}” in {1} s", subject, secs)
}

/// The composer's labels and names.
pub mod words {
    use komorebi_i18n::tr;

    pub fn from() -> String {
        tr!("From")
    }
    pub fn to() -> String {
        tr!("To")
    }
    pub fn cc() -> String {
        tr!("Cc")
    }
    pub fn bcc() -> String {
        tr!("Bcc")
    }
    pub fn subject() -> String {
        tr!("Subject")
    }
    pub fn send() -> String {
        tr!("Send")
    }
    pub fn send_tip() -> String {
        tr!("Send (Ctrl+Enter)")
    }
    pub fn send_anyway() -> String {
        tr!("Send anyway")
    }
    pub fn keep_editing() -> String {
        tr!("Keep editing")
    }
    pub fn discard() -> String {
        tr!("Discard")
    }
    pub fn attach() -> String {
        tr!("Attach")
    }
    /// The address suggestions under a field.
    pub fn suggested_recipients() -> String {
        tr!("Suggested recipients")
    }
    /// Over the composer while files are dragged onto it.
    pub fn drop_to_attach() -> String {
        tr!("Drop to attach")
    }
    pub fn attach_tip() -> String {
        tr!("Attach (Ctrl+Shift+A)")
    }
    pub fn close_tip() -> String {
        tr!("Close (Esc)")
    }
    pub fn add_cc() -> String {
        tr!("Add Cc (Ctrl+Shift+C)")
    }
    pub fn add_bcc() -> String {
        tr!("Add Bcc (Ctrl+Shift+B)")
    }
    pub fn body_name() -> String {
        tr!("Message body")
    }
    pub fn body_placeholder() -> String {
        tr!("Write your message")
    }
    pub fn remove(name: &str) -> String {
        tr!("Remove {0}", name)
    }
    pub fn signature() -> String {
        tr!("Signature")
    }
    pub fn signature_none() -> String {
        tr!("signature" => "None")
    }
    pub fn manage_signatures() -> String {
        tr!("Manage signatures…")
    }
    pub fn link() -> String {
        tr!("Link")
    }
    pub fn link_tip() -> String {
        tr!("Link (Ctrl+K)")
    }
    pub fn link_address() -> String {
        tr!("Link address")
    }
    pub fn link_placeholder() -> String {
        tr!("https://example.com or name@example.com")
    }
    pub fn link_bad() -> String {
        tr!("Not a web or e-mail address")
    }
    pub fn insert() -> String {
        tr!("Insert")
    }
    pub fn insert_link_tip() -> String {
        tr!("Insert link (Enter)")
    }
    pub fn cancel_tip() -> String {
        tr!("Cancel (Esc)")
    }
    pub fn bold() -> String {
        tr!("Bold (Ctrl+B)")
    }
    pub fn italic() -> String {
        tr!("Italic (Ctrl+I)")
    }
    pub fn underline() -> String {
        tr!("Underline (Ctrl+U)")
    }
    pub fn strike() -> String {
        tr!("Strikethrough (Ctrl+Shift+X)")
    }
    pub fn bullets() -> String {
        tr!("Bulleted list (Ctrl+Shift+8)")
    }
    pub fn numbers() -> String {
        tr!("Numbered list (Ctrl+Shift+7)")
    }
    pub fn outdent() -> String {
        tr!("Decrease indent")
    }
    pub fn indent() -> String {
        tr!("Increase indent")
    }
    pub fn quote() -> String {
        tr!("Quote")
    }
    pub fn clear() -> String {
        tr!("Clear formatting")
    }
    pub fn insert_table() -> String {
        tr!("Insert table")
    }
    pub fn insert_picture() -> String {
        tr!("Insert picture")
    }
    pub fn picture() -> String {
        tr!("Picture")
    }
    pub fn alt_text() -> String {
        tr!("Alt text…")
    }
    pub fn alt_text_field() -> String {
        tr!("Alt text")
    }
    pub fn picture_bigger() -> String {
        tr!("Make bigger (Ctrl+])")
    }
    pub fn picture_smaller() -> String {
        tr!("Make smaller (Ctrl+[)")
    }
    pub fn picture_own_size() -> String {
        tr!("Original size")
    }
    pub fn use_original() -> String {
        tr!("Use original picture")
    }
    pub fn delete_table() -> String {
        tr!("Delete table")
    }
    pub fn table_op(op: komorebi::mail::rich::edit::TableOp) -> String {
        use komorebi::mail::rich::edit::TableOp;
        match op {
            TableOp::RowAbove => tr!("Insert row above"),
            TableOp::RowBelow => tr!("Insert row below"),
            TableOp::ColLeft => tr!("Insert column left"),
            TableOp::ColRight => tr!("Insert column right"),
            TableOp::DeleteRow => tr!("Delete row"),
            TableOp::DeleteCol => tr!("Delete column"),
            TableOp::DeleteTable => tr!("Delete table"),
        }
    }
    pub fn table() -> String {
        tr!("Table")
    }
    pub fn font() -> String {
        tr!("Font")
    }
    pub fn font_size() -> String {
        tr!("Font size")
    }
    pub fn font_colour() -> String {
        tr!("Font colour")
    }
    pub fn colour(c: Option<komorebi::mail::rich::ColorId>) -> String {
        use komorebi::mail::rich::ColorId;
        match c {
            None => tr!("colour" => "Automatic"),
            Some(ColorId::Black) => tr!("Black"),
            Some(ColorId::Grey) => tr!("Grey"),
            Some(ColorId::DarkRed) => tr!("Dark red"),
            Some(ColorId::Red) => tr!("Red"),
            Some(ColorId::Orange) => tr!("Orange"),
            Some(ColorId::Gold) => tr!("Gold"),
            Some(ColorId::Green) => tr!("Green"),
            Some(ColorId::Teal) => tr!("Teal"),
            Some(ColorId::Blue) => tr!("Blue"),
            Some(ColorId::Purple) => tr!("Purple"),
            Some(ColorId::Rgb(..)) => tr!("Custom colour"),
        }
    }
    pub fn high() -> String {
        tr!("High importance")
    }
    pub fn low() -> String {
        tr!("Low importance")
    }
    pub fn more() -> String {
        tr!("More options")
    }
    pub fn to_plain() -> String {
        tr!("Switch to plain text")
    }
    pub fn to_formatted() -> String {
        tr!("Switch to formatted text")
    }
    pub fn preview() -> String {
        tr!("Preview")
    }
    pub fn preview_failed() -> String {
        tr!("Couldn't draw the preview. The message is sent as written.")
    }
    pub fn show_quote() -> String {
        tr!("Show quoted text")
    }
    pub fn edit_quote() -> String {
        tr!("Edit quoted text")
    }
    pub fn align_left() -> String {
        tr!("Align left")
    }
    pub fn align_center() -> String {
        tr!("Center")
    }
    pub fn align_right() -> String {
        tr!("Align right")
    }
    pub fn justify() -> String {
        tr!("Justify")
    }
    /// The format bar as a whole (UI Automation's toolbar name).
    pub fn formatting() -> String {
        tr!("Formatting")
    }
    pub fn alignment() -> String {
        tr!("Alignment")
    }
    /// A box's value when the selection holds more than one.
    pub fn mixed() -> String {
        tr!("format value" => "Mixed")
    }
    /// A box's accessible name: its title and the value it shows.
    pub fn valued(title: &str, value: &str) -> String {
        tr!("{0}: {1}", title, value)
    }
    pub fn font_size_tip() -> String {
        tr!("Font size (Ctrl+[ or Ctrl+])")
    }
    pub fn indent_tip() -> String {
        tr!("Increase indent (Ctrl+M)")
    }
    pub fn outdent_tip() -> String {
        tr!("Decrease indent (Ctrl+Shift+M)")
    }
    pub fn quote_tip() -> String {
        tr!("Quote (Ctrl+Shift+Q)")
    }
    pub fn clear_tip() -> String {
        tr!("Clear formatting (Ctrl+Space)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use komorebi_i18n::with_lang;

    /// Word for word the Slint app's sentences (src/app/text.rs and
    /// ui/tr.slint), and their European Portuguese from the catalog.
    #[test]
    fn the_composer_says_what_the_slint_app_says() {
        with_lang(Lang::En, || {
            assert_eq!(title(Kind::ReplyAll), "Reply all");
            assert_eq!(
                send_block(SendBlock::NoRecipient),
                "Add a recipient to send."
            );
            assert_eq!(
                send_block(SendBlock::FixAddress(Field::Cc)),
                "Fix the Cc address to send."
            );
            assert_eq!(guard(Guard::NoSubject), "This message has no subject.");
            assert_eq!(address_invalid("j@"), "“j@” is not a valid address");
            assert_eq!(attach_total("3 MB"), "Attachments: 3 MB");
            let fail = AttachFail {
                name: "a".into(),
                why: AttachWhy::Folder,
            };
            assert_eq!(attach_fail(&fail), "a: folders can't be attached");
            let h = heads(Lang::En);
            assert_eq!((h.wrote)("Mon", "Bob"), "On Mon, Bob wrote:");
            assert_eq!(h.forwarded, komorebi::compose::FORWARDED);
        });
        with_lang(Lang::PtPt, || {
            assert_eq!(address_invalid("j@"), "«j@» não é um endereço válido");
            assert_ne!(title(Kind::New), "New message");
            assert_ne!(words::send(), "Send");
            let h = heads(Lang::PtPt);
            assert_ne!(h.forwarded, komorebi::compose::FORWARDED);
        });
    }

    #[test]
    fn sizes_as_the_slint_app_shows_them() {
        assert_eq!(size(Lang::En, 512), "512 B");
        assert_eq!(size(Lang::En, 12 * 1024), "12 KB");
        assert_eq!(size(Lang::PtPt, 3 * 1024 * 1024 / 2), "1,5\u{a0}MB");
    }
}
