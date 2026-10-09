//! Shared helpers for the HTML renderer tests: load a synthetic corpus
//! message (`tests/fixtures/html/`) into a `RenderInput`.

#![allow(dead_code)]

use komorebi::mail::mime;
use komorebi::mail::render::RenderInput;
use std::path::PathBuf;

pub const FILES: [&str; 14] = [
    "01-mailchimp-campaign",
    "02-substack-post",
    "03-github-review",
    "04-order-receipt",
    "05-bank-statement",
    "06-outlook-word",
    "07-gmail-reply-chain",
    "08-dark-marketing",
    "09-image-heavy-promo",
    "10-transactional-buttons",
    "11-newsletter-2col",
    "12-personal-applemail",
    "13-calendar-invite",
    "14-adversarial-malformed",
];

pub fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/html")
        .join(name)
}

pub fn raw(name: &str) -> Vec<u8> {
    std::fs::read(fixture(&format!("{name}.eml"))).unwrap()
}

/// What the reader would hand the renderer for this message.
pub fn input(raw: &[u8]) -> RenderInput {
    mime::parse_render_input(raw)
}

/// Run `f` on a thread with the render worker's 8 MiB stack.
pub fn big_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new()
        .stack_size(8 << 20)
        .spawn(f)
        .unwrap()
        .join()
        .unwrap()
}
