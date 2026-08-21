//! Shared rendering helpers and formatting functions used across the UI.
//!
//! These are free functions that don't depend on `App` state - they take
//! explicit arguments and the theme/context they need.

use crate::db::{Attachment, Database, MessageDetail};
use crate::mail;
use crate::ui::App;
use crate::ui::theme::komorebi_hex;
use gpui::{AnyElement, Context, Hsla, IntoElement, ParentElement, SharedString, Styled, div, px};
use gpui_component::{ActiveTheme as _, Icon, IconName, StyledExt as _, h_flex, v_flex};
use std::path::{Path, PathBuf};

/// Color for a user label from its stored hex (`"6b9bd6"`, `"#6b9bd6"`),
/// falling back to the name-based palette when unset/malformed.
pub fn stored_label_color(hex: Option<&str>, fallback: &str) -> Hsla {
    if let Some(hex) = hex {
        let t = hex.trim().trim_start_matches('#');
        if t.len() == 6
            && let Ok(v) = u32::from_str_radix(t, 16)
        {
            return komorebi_hex(v);
        }
    }
    label_color(fallback)
}

// ---------------------------------------------------------------------------
// Badge / tag helpers
// ---------------------------------------------------------------------------

/// A small colored pill shown on message rows and in AI insights.
pub fn label_badge(cx: &mut Context<App>, label: &str) -> AnyElement {
    let color = label_color(label);
    h_flex()
        .flex_none()
        .gap_1()
        .px_1p5()
        .py_0p5()
        .rounded_full()
        .bg(cx.theme().muted)
        .items_center()
        .child(
            div()
                .w(px(6.0))
                .h(px(6.0))
                .rounded_full()
                .bg(color)
                .flex_none(),
        )
        .child(
            div()
                .text_xs()
                .text_color(color)
                .child(SharedString::from(label.to_string())),
        )
        .into_any_element()
}

pub fn label_color(label: &str) -> Hsla {
    let hex = match label.to_lowercase().as_str() {
        "work" => 0x6b9bd6,
        "design" => 0xa58bd6,
        "personal" => 0xa8c256,
        "finance" => 0xd68b4e,
        "travel" => 0xd66b5e,
        "clients" => 0x6b9bd6,
        "product" => 0x7e9a6b,
        "follow up" | "follow-up" => 0xc99a4a,
        _ => 0x9a9284,
    };
    komorebi_hex(hex)
}

/// Category + optional "Follow up" tag shown beside the reader subject.
pub fn category_tags(
    cx: &mut Context<App>,
    category: Option<&str>,
    flagged: bool,
) -> Vec<AnyElement> {
    let mut tags = Vec::new();
    if let Some(cat) = category {
        tags.push(label_badge(cx, cat));
    }
    if flagged {
        tags.push(label_badge(cx, "Follow up"));
    }
    tags
}

/// A calendar tag for an active snooze or follow-up reminder.
pub fn reminder_tag(cx: &mut Context<App>, label: &str) -> AnyElement {
    h_flex()
        .flex_none()
        .gap_1()
        .px_1p5()
        .py_0p5()
        .rounded_full()
        .bg(cx.theme().muted)
        .items_center()
        .child(
            Icon::new(IconName::Calendar)
                .size_3p5()
                .text_color(cx.theme().warning),
        )
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().warning)
                .child(SharedString::from(label.to_string())),
        )
        .into_any_element()
}

// ---------------------------------------------------------------------------
// File-type helpers
// ---------------------------------------------------------------------------

pub fn file_type_style(name: &str) -> (Hsla, &'static str) {
    let ext = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "pdf" => (komorebi_hex(0xc9684e), "PDF"),
        "xlsx" | "xls" => (komorebi_hex(0x7e9a6b), "XLS"),
        "docx" | "doc" => (komorebi_hex(0x6b7fa0), "DOC"),
        "png" | "jpg" | "jpeg" | "gif" => (komorebi_hex(0xa58bd6), "IMG"),
        "zip" => (komorebi_hex(0xc99a4a), "ZIP"),
        _ => (komorebi_hex(0x9a9284), "FILE"),
    }
}

// ---------------------------------------------------------------------------
// Recipient / thread text helpers
// ---------------------------------------------------------------------------

/// A single-line recipient hint: "to me" or "to {first recipient}".
pub fn recipient_line(to_json: &str, sender_email: &str) -> String {
    let recipients: Vec<String> = serde_json::from_str(to_json).unwrap_or_default();
    match recipients.first() {
        Some(r) if r == sender_email => "to me".to_string(),
        Some(r) => format!("to {r}"),
        None => "to me".to_string(),
    }
}

/// Format a whole conversation as per-message blocks for the AI summarize
/// action (sender + subject + body per message, oldest first).
pub fn format_thread_text(messages: &[MessageDetail]) -> String {
    messages
        .iter()
        .map(|m| {
            let sender = m
                .sender_name
                .clone()
                .unwrap_or_else(|| m.sender_email.clone().unwrap_or_default());
            let subject = m
                .subject
                .clone()
                .unwrap_or_else(|| "(no subject)".to_string());
            let body = m.body_text.clone().unwrap_or_default();
            format!("From: {sender}\nSubject: {subject}\n\n{}", body.trim())
        })
        .collect::<Vec<_>>()
        .join("\n\n---\n\n")
}

// ---------------------------------------------------------------------------
// Body rendering
// ---------------------------------------------------------------------------

/// Render plain text as line-separated blocks (the demo bodies use `\n\n`).
pub fn render_body(body: &str) -> AnyElement {
    // Japandi reading rhythm: generous paragraph spacing for calm readability.
    let mut col = v_flex().gap_2();
    for para in body.split("\n\n") {
        let trimmed = para.trim();
        if trimmed.is_empty() {
            continue;
        }
        let mut para_col = v_flex().gap_0p5();
        for line in trimmed.lines() {
            para_col = para_col.child(div().text_sm().child(SharedString::from(line.to_string())));
        }
        col = col.child(para_col);
    }
    col.into_any_element()
}

/// Render a message body, preferring sanitized HTML when available.
/// `allow_remote` is the per-message consent flag from the reader.
pub fn render_message_body(
    body_text: &str,
    body_html: Option<&str>,
    allow_remote: bool,
    _cx: &mut Context<App>,
) -> AnyElement {
    if let Some(html) = body_html.filter(|h| !h.trim().is_empty()) {
        let sanitized = mail::html::sanitize(html, allow_remote);
        // If the sanitized HTML is just whitespace/tags with no visible text,
        // fall back to plain text so the reader never shows an empty body.
        let stripped = mail::mime::strip_html(&sanitized.html);
        if stripped.trim().is_empty() && !body_text.trim().is_empty() {
            return render_body(body_text);
        }
        let mut col = v_flex().gap_2();
        // Render the sanitized HTML as preformatted text. GPUI does not have a
        // native HTML view; we render the stripped text with structure preserved
        // by the sanitizer. This keeps remote content blocked by default.
        let text = mail::mime::strip_html(&sanitized.html);
        for para in text.split("\n\n") {
            let trimmed = para.trim();
            if trimmed.is_empty() {
                continue;
            }
            let mut para_col = v_flex().gap_0p5();
            for line in trimmed.lines() {
                if !line.trim().is_empty() {
                    para_col =
                        para_col.child(div().text_sm().child(SharedString::from(line.to_string())));
                }
            }
            col = col.child(para_col);
        }
        col.into_any_element()
    } else {
        render_body(body_text)
    }
}

// ---------------------------------------------------------------------------
// AI insights helper
// ---------------------------------------------------------------------------

pub fn insight_row(cx: &mut Context<App>, label: &str, value: Option<AnyElement>) -> AnyElement {
    h_flex()
        .w_full()
        .px_2()
        .items_center()
        .justify_between()
        .child(
            div()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(SharedString::from(label.to_string())),
        )
        .children(value)
        .into_any_element()
}

// ---------------------------------------------------------------------------
// Date / time formatters
// ---------------------------------------------------------------------------

pub fn format_list_date(ts: Option<i64>) -> String {
    let Some(t) = ts else {
        return String::new();
    };
    let Some(dt) = chrono::DateTime::from_timestamp(t, 0) else {
        return String::new();
    };
    let diff = chrono::Utc::now().timestamp() - t;
    if diff < 60 {
        "now".into()
    } else if diff < 3600 {
        format!("{}m", diff / 60)
    } else if diff < 86400 {
        format!("{}h", diff / 3600)
    } else if diff < 7 * 86400 {
        dt.format("%a").to_string()
    } else {
        dt.format("%b %d").to_string()
    }
}

pub fn format_full_date(ts: Option<i64>) -> String {
    match ts {
        Some(t) => chrono::DateTime::from_timestamp(t, 0)
            .map(|d| d.format("%b %d, %Y · %H:%M").to_string())
            .unwrap_or_default(),
        None => String::new(),
    }
}

pub fn format_relative_long(ts: i64) -> String {
    let diff = chrono::Utc::now().timestamp() - ts;
    if diff < 60 {
        "just now".to_string()
    } else if diff < 3600 {
        format!("{}m ago", diff / 60)
    } else if diff < 86400 {
        let h = diff / 3600;
        let m = (diff % 3600) / 60;
        format!("{h}h {m:02}m")
    } else {
        format!("{}d ago", diff / 86400)
    }
}

pub fn format_bytes(bytes: i64) -> String {
    if bytes >= 1_048_576 {
        format!("{:.1} MB", bytes as f64 / 1_048_576.0)
    } else if bytes >= 1024 {
        format!("{:.0} KB", bytes as f64 / 1024.0)
    } else {
        format!("{bytes} B")
    }
}

// ---------------------------------------------------------------------------
// List decorators
// ---------------------------------------------------------------------------

/// A compact unread-count badge, or nothing when there are no unread messages.
pub fn unread_guard(unread: i64, cx: &mut Context<App>) -> Option<AnyElement> {
    if unread <= 0 {
        return None;
    }
    Some(
        div()
            .text_xs()
            .font_medium()
            .px_1p5()
            .rounded_full()
            .bg(cx.theme().sidebar_primary)
            .text_color(cx.theme().sidebar_primary_foreground)
            .child(SharedString::from(unread.to_string()))
            .into_any_element(),
    )
}

/// A small star marker shown for flagged messages in the list.
pub fn star_marker(flagged: bool, cx: &mut Context<App>) -> Option<AnyElement> {
    if !flagged {
        return None;
    }
    Some(
        Icon::new(IconName::Star)
            .size_3p5()
            .text_color(cx.theme().primary)
            .into_any_element(),
    )
}

// ---------------------------------------------------------------------------
// Attachment file helpers
// ---------------------------------------------------------------------------

pub fn load_attachment_data(db: &Database, att: &Attachment) -> anyhow::Result<Vec<u8>> {
    db.get_attachment_data(&att.id)?
        .ok_or_else(|| anyhow::anyhow!("attachment data missing for {}", att.display_name))
}

/// Sanitize a filename so it can be safely written to disk (no path
/// separators, control characters, or Windows-invalid characters).
pub fn sanitize_filename(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') {
                '_'
            } else {
                c
            }
        })
        .collect();
    let cleaned = cleaned.trim().trim_matches('.').to_string();
    if cleaned.is_empty() {
        "attachment".to_string()
    } else {
        cleaned
    }
}

/// Find a non-colliding path in `dir` for `name`, appending " (n)" when needed.
pub fn unique_path(dir: &Path, name: &str) -> PathBuf {
    let name = sanitize_filename(name);
    let candidate = dir.join(&name);
    if !candidate.exists() {
        return candidate;
    }
    let (stem, ext) = match name.rsplit_once('.') {
        Some((stem, ext)) => (stem.to_string(), format!(".{ext}")),
        None => (name.clone(), String::new()),
    };
    for i in 1.. {
        let candidate = dir.join(format!("{stem} ({i}){ext}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    unreachable!()
}

/// Write raw bytes to `dir` under a safe, collision-free filename.
pub fn write_blob(dir: &Path, name: &str, data: &[u8]) -> anyhow::Result<PathBuf> {
    let path = unique_path(dir, name);
    std::fs::write(&path, data)?;
    Ok(path)
}

pub fn is_image_name(name: &str) -> bool {
    matches!(
        name.rsplit('.')
            .next()
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some(
            "png"
                | "jpg"
                | "jpeg"
                | "gif"
                | "webp"
                | "bmp"
                | "ico"
                | "svg"
                | "avif"
                | "tif"
                | "tiff"
        )
    )
}

pub fn is_pdf_name(name: &str) -> bool {
    name.rsplit('.')
        .next()
        .map(str::to_ascii_lowercase)
        .as_deref()
        == Some("pdf")
}

/// Open the on-disk cache, falling back to an in-memory database if the config
/// directory or file is unavailable.
pub fn open_database() -> Database {
    if let Some(dir) = dirs::config_dir() {
        let path = dir.join("komorebi").join("cache.db");
        if let Ok(db) = Database::open(&path) {
            return db;
        }
    }
    Database::in_memory().expect("in-memory database must open")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_filename_strips_dangerous_chars() {
        assert_eq!(sanitize_filename("plain.txt"), "plain.txt");
        assert_eq!(sanitize_filename("../../etc/passwd"), "_.._etc_passwd");
        assert_eq!(sanitize_filename("a<b>:c|d?e*"), "a_b__c_d_e_");
        assert_eq!(sanitize_filename("..."), "attachment");
    }

    #[test]
    fn unique_path_appends_suffix_on_collision() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("note.txt"), b"x").unwrap();
        let p = unique_path(dir.path(), "note.txt");
        assert_eq!(p.file_name().unwrap().to_str().unwrap(), "note (1).txt");
        assert!(!p.exists());
    }

    #[test]
    fn file_kind_detection() {
        assert!(is_image_name("photo.PNG"));
        assert!(is_image_name("gif.gif"));
        assert!(is_pdf_name("invoice.pdf"));
        assert!(!is_pdf_name("photo.png"));
        assert!(!is_image_name("notes.txt"));
    }
}
