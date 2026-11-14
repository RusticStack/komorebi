//! Attachments: reading files to attach within Komorebi's limits, and the
//! counter that says how close a message is to what servers take.

use std::path::{Path, PathBuf};

use super::Attached;
use crate::mail::smtp::OutAttachment;

/// Encoded attachment size past which many servers reject a message.
pub const MAX_ATTACHMENTS_BYTES: u64 = 25 * 1024 * 1024;
/// Komorebi's own limit on encoded attachments, checked before reading.
pub const MAX_ATTACH_HARD: u64 = 100 * 1024 * 1024;

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
    /// Past Komorebi's 100 MB limit.
    TooBig,
    /// The file system's words.
    Io(String),
}

/// Read files to attach when `total` bytes are attached already: the
/// attachments, and what went wrong with each file that can't be.
pub fn read_files(paths: &[PathBuf], mut total: u64) -> (Vec<Attached>, Vec<AttachFail>) {
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
        match read_attachment(path, total) {
            Ok(data) => {
                total += data.len() as u64;
                added.push(Attached::from(OutAttachment {
                    filename: name,
                    data,
                }));
            }
            Err(why) => errors.push(AttachFail { name, why }),
        }
    }
    (added, errors)
}

/// The attachment counter's level: 0 normal, 1 near the limit (from 80%),
/// 2 over it (Send is off).
pub fn counter_level(encoded: u64) -> i32 {
    if encoded > MAX_ATTACHMENTS_BYTES {
        2
    } else if encoded.saturating_mul(5) >= MAX_ATTACHMENTS_BYTES * 4 {
        1
    } else {
        0
    }
}

pub fn attachments_size(list: &[Attached]) -> u64 {
    list.iter().map(|a| a.file.data.len() as u64).sum()
}

/// Size once base64-encoded for sending.
pub fn encoded_size(raw: u64) -> u64 {
    raw.saturating_mul(4) / 3
}

/// Read a file to attach when `total` bytes are attached already. A file that
/// would take the message past the hard limit is refused before reading.
pub fn read_attachment(path: &Path, total: u64) -> Result<Vec<u8>, AttachWhy> {
    use std::io::Read;
    let io = |e: std::io::Error| AttachWhy::Io(e.to_string());
    let len = std::fs::metadata(path).map_err(io)?.len();
    if encoded_size(total.saturating_add(len)) > MAX_ATTACH_HARD {
        return Err(AttachWhy::TooBig);
    }
    // The file may grow after the size check; never read past the limit.
    let limit = (MAX_ATTACH_HARD * 3 / 4).saturating_sub(total);
    let mut data = Vec::with_capacity(len as usize);
    std::fs::File::open(path)
        .map_err(io)?
        .take(limit + 1)
        .read_to_end(&mut data)
        .map_err(io)?;
    if data.len() as u64 > limit {
        return Err(AttachWhy::TooBig);
    }
    Ok(data)
}
