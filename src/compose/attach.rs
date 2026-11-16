//! Attachments: reading files to attach, and their total size.
//!
//! Komorebi sets no limit on how big attachments can be (owner decision,
//! 2026-11-16): the account's server is the only judge, and its own "too
//! large" refusal (`mail::smtp`, 552 or 5.3.4) is what the user hears.

use std::path::{Path, PathBuf};

use super::Attached;
use crate::mail::smtp::OutAttachment;

/// A file that could not be attached, and why.
#[derive(Debug, PartialEq, Eq)]
pub struct AttachFail {
    pub name: String,
    pub why: AttachWhy,
}

#[derive(Debug, PartialEq, Eq)]
pub enum AttachWhy {
    /// A folder.
    Folder,
    /// The file system's words.
    Io(String),
}

/// Read files to attach: the attachments, and what went wrong with each
/// file that can't be. Each file is read once, whole (call it off the UI
/// thread).
pub fn read_files(paths: &[PathBuf]) -> (Vec<Attached>, Vec<AttachFail>) {
    let mut added = Vec::new();
    let mut errors = Vec::new();
    for path in paths {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "attachment".into());
        if path.is_dir() {
            errors.push(AttachFail {
                name,
                why: AttachWhy::Folder,
            });
            continue;
        }
        match read_attachment(path) {
            Ok(data) => added.push(Attached::from(OutAttachment {
                filename: name,
                data,
            })),
            Err(why) => errors.push(AttachFail { name, why }),
        }
    }
    (added, errors)
}

/// The attached files' size, as they are on disk.
pub fn attachments_size(list: &[Attached]) -> u64 {
    list.iter().map(|a| a.file.data.len() as u64).sum()
}

/// Read a file to attach, whole, in one pass (`std::fs::read` sizes its
/// buffer from the file's length, so a large file is not copied as the
/// buffer grows).
pub fn read_attachment(path: &Path) -> Result<Vec<u8>, AttachWhy> {
    std::fs::read(path).map_err(|e| AttachWhy::Io(e.to_string()))
}
