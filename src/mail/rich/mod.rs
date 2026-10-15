//! The composer's own document model for formatted mail and signatures.
//!
//! Komorebi never sends HTML the user typed or pasted as it is. The editor
//! (not built yet) changes a small [`Doc`]; this module generates the
//! text/plain and text/html parts from it ([`emit`]) and reads back only
//! its own stored HTML ([`import::strict`]) or, for pasted, quoted and
//! foreign-draft HTML, a lossy mapping of sanitized markup
//! ([`import::lossy`]). Fonts, sizes and colours are closed sets, so no free
//! CSS reaches the output; nothing here loads anything.
//!
//! The input can be hostile (pasted or server HTML), so the module denies
//! the lints below, every scan is linear, and the caps in [`model`] bound
//! the size of a document. A failure degrades to plain text.

#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

pub mod edit;
pub mod emit;
pub mod import;
pub mod logo;
pub mod model;
pub mod ops;
pub mod view;

pub use emit::{Mode, html, needs_html, plain};
pub use logo::{Logo, LogoError, MAX_LOGO_BYTES};
pub use model::*;

/// The two parts of a message made from `doc`: the text/plain body and,
/// when the document needs one, the text/html alternative. A document with
/// no formatting is plain text only.
pub fn parts(doc: &Doc) -> (String, Option<String>) {
    let text = plain(doc);
    let html = needs_html(doc).then(|| html(doc, Mode::Send));
    (text, html)
}

/// A document read back from stored HTML: strictly when it carries
/// Komorebi's tag, otherwise (a draft another client saved) through the
/// lossy mapping. `None` if nothing readable is left.
pub fn from_stored(html: &str) -> Option<Doc> {
    import::strict(html).or_else(|| import::lossy(html))
}

/// The followable form of a link address the user typed: `http`, `https`
/// and `mailto` kept, a bare host or path given `https://`, an e-mail
/// address `mailto:`, any other scheme refused. Brackets and parentheses
/// are percent-encoded. `None` if it cannot be followed.
pub fn link_target(url: &str) -> Option<String> {
    use super::render::links;
    let url = url.trim();
    if url.is_empty() || url.len() > model::MAX_LINK_BYTES || url.contains(char::is_whitespace) {
        return None;
    }
    let scheme = url
        .split_once(':')
        .map(|(s, _)| s)
        .filter(|s| !s.is_empty() && s.bytes().all(|c| c.is_ascii_alphabetic()));
    let full = match scheme {
        Some(_) => url.to_string(),
        None if url.contains('@') && !url.contains('/') => format!("mailto:{url}"),
        None => format!("https://{url}"),
    };
    let clean = links::sanitize(&full)?;
    if clean.starts_with('#') {
        return None;
    }
    let mut out = String::with_capacity(clean.len());
    for c in clean.chars() {
        match c {
            '(' => out.push_str("%28"),
            ')' => out.push_str("%29"),
            '[' => out.push_str("%5B"),
            ']' => out.push_str("%5D"),
            c => out.push(c),
        }
    }
    (out.len() <= model::MAX_LINK_BYTES).then_some(out)
}

#[cfg(test)]
mod edit_tests;
#[cfg(test)]
mod tests;
