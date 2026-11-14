//! The reader's display formatting: the library's `komorebi::format` (the
//! sender, the recipients, the date with Windows' short date, an
//! attachment's size), plus the attachment kind as a type.

pub use komorebi::format::{reader_date, recipients, sender, size};

/// What an attachment is, from its file name's extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileKind {
    File,
    Picture,
    Document,
    Calendar,
    Message,
}

/// [`komorebi::format::file_kind`] as a [`FileKind`].
pub fn file_kind(name: &str) -> FileKind {
    match komorebi::format::file_kind(name) {
        1 => FileKind::Picture,
        2 => FileKind::Document,
        3 => FileKind::Calendar,
        4 => FileKind::Message,
        _ => FileKind::File,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use komorebi::i18n::Lang;

    #[test]
    fn as_the_slint_app_formats_them() {
        assert_eq!(size(Lang::En, 512), "512 B");
        assert_eq!(size(Lang::PtPt, 1_258_291), "1,2\u{a0}MB");
        assert_eq!(file_kind("geres-lake.JPG"), FileKind::Picture);
        assert_eq!(file_kind("invite.ics"), FileKind::Calendar);
        assert_eq!(file_kind("notes"), FileKind::File);
        assert_eq!(sender("  ", "a@x"), "a@x");
        assert_eq!(
            recipients("Ana <a@x>, \"Doe, Jane\" <j@x>, b@x"),
            "Ana, Doe, Jane, b@x"
        );
        assert_eq!(reader_date(Lang::En, 0), "");
    }
}
