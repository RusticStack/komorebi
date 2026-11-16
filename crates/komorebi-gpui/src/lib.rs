//! Komorebi 1.0 on GPUI (docs/GPUI-MIGRATION.md): the app's modules, shared
//! by the binary (`main.rs`), the component gallery and the tests.

pub mod access;
#[cfg(feature = "alloc-count")]
pub mod alloc_count;
pub mod backdrop;
pub mod bench;
#[cfg(feature = "gallery")]
pub mod gallery;
pub mod i18n;
pub mod icons;
pub mod preflight;
#[cfg(feature = "sampler")]
pub mod sampler;
pub mod snapshot;
pub mod snapshot_drive;
pub mod tabs;
#[cfg(test)]
pub(crate) mod test_support;
pub mod text;
pub mod theme;
pub mod titlebar;
pub mod tray;
pub mod views;
pub mod wake;
pub mod widgets;
pub mod window;
