//! The Komorebi additions to the vendored GPUI (third_party/gpui-pre/KOMOREBI-PATCH.md):
//! the vendored crates are not in the workspace, so their own unit tests do not run in
//! `cargo test`; what the app relies on is checked here, through the public API.

use std::sync::Arc;

use gpui::{DevicePixels, RenderImage, Size, size};
use image::Delay;

/// `RenderImage::from_shared_bgra` reads the caller's buffer without a copy, like one frame of
/// `RenderImage::new`, and refuses a buffer of the wrong length.
#[test]
fn a_shared_image_reads_the_callers_buffer_without_a_copy() {
    let bytes: Arc<[u8]> = Arc::from(vec![7u8; 3 * 2 * 4]);
    let image = RenderImage::from_shared_bgra(3, 2, bytes.clone()).expect("3 x 2 x 4 bytes");
    assert_eq!(image.frame_count(), 1);
    assert_eq!(image.size(0), size(DevicePixels(3), DevicePixels(2)));
    assert_eq!(image.size(1), Size::default());
    assert_eq!(image.delay(0), Delay::from_numer_denom_ms(0, 1));
    assert_eq!(image.as_bytes(1), None);
    let read = image.as_bytes(0).expect("frame 0");
    assert_eq!(read, &bytes[..]);
    assert!(std::ptr::eq(read.as_ptr(), bytes.as_ptr()), "no copy");
    assert!(RenderImage::from_shared_bgra(3, 2, Arc::from(vec![0u8; 23])).is_none());
    assert!(format!("{image:?}").contains("(3, 2)"));

    // The owning form is unchanged and reports the same way.
    let owned = RenderImage::new(vec![image::Frame::new(
        image::RgbaImage::from_raw(3, 2, vec![7u8; 24]).expect("3 x 2"),
    )]);
    assert_eq!(owned.frame_count(), 1);
    assert_eq!(owned.size(0), image.size(0));
    assert_eq!(owned.delay(0), image.delay(0));
    assert_eq!(owned.as_bytes(0), image.as_bytes(0));
}

/// The pre-flight hands its device to GPUI: on WARP (every Windows has it, GPU or not) the
/// devices are made once on the calling thread, off the UI thread, and kept for the platform.
#[cfg(windows)]
#[test]
fn devices_are_prepared_on_another_thread() {
    let made = std::thread::spawn(|| gpui_windows::prepare_devices(true))
        .join()
        .expect("the thread did not panic");
    assert_eq!(made, Ok(()));
}
