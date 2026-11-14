use crate::{DevicePixels, Pixels, Result, SharedString, Size, size};
use smallvec::SmallVec;

use image::{Delay, Frame};
use std::{
    borrow::Cow,
    fmt,
    hash::Hash,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering::SeqCst},
    },
};

/// A source of assets for this app to use.
pub trait AssetSource: 'static + Send + Sync {
    /// Load the given asset from the source path.
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>>;

    /// List the assets at the given path.
    fn list(&self, path: &str) -> Result<Vec<SharedString>>;
}

impl AssetSource for () {
    fn load(&self, _path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        Ok(None)
    }

    fn list(&self, _path: &str) -> Result<Vec<SharedString>> {
        Ok(vec![])
    }
}

/// A unique identifier for the image cache
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct ImageId(pub usize);

#[derive(PartialEq, Eq, Hash, Clone)]
#[expect(missing_docs)]
pub struct RenderImageParams {
    pub image_id: ImageId,
    pub frame_index: usize,
}

/// A cached and processed image, in BGRA format
pub struct RenderImage {
    /// The ID associated with this image
    pub id: ImageId,
    /// The scale factor of this image on render.
    pub(crate) scale_factor: f32,
    data: SmallVec<[Frame; 1]>,
    // KOMOREBI PATCH (shared-pixels): a single frame that shares the caller's pixel buffer
    // instead of owning a copy (`data` is then empty).
    shared: Option<SharedFrame>,
}

/// KOMOREBI PATCH (shared-pixels): one frame whose pixels are a buffer the caller also holds.
struct SharedFrame {
    width: u32,
    height: u32,
    bytes: Arc<[u8]>,
}

impl PartialEq for RenderImage {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl Eq for RenderImage {}

impl RenderImage {
    /// Create a new image from the given data.
    pub fn new(data: impl Into<SmallVec<[Frame; 1]>>) -> Self {
        static NEXT_ID: AtomicUsize = AtomicUsize::new(0);

        Self {
            id: ImageId(NEXT_ID.fetch_add(1, SeqCst)),
            scale_factor: 1.0,
            data: data.into(),
            shared: None,
        }
    }

    /// KOMOREBI PATCH (shared-pixels): creates a one-frame image over a pixel buffer the caller
    /// keeps sharing, with no copy. `bytes` is `width` x `height` pixels of BGRA (premultiplied,
    /// rows top to bottom), as the frames of [`RenderImage::new`] are, so `width * height * 4`
    /// bytes. `None` when the length does not match. A decoded tile cache can hand the same
    /// `Arc<[u8]>` to the image, so the pixels exist once on the CPU side; the GPU atlas still
    /// holds its own copy once the image is painted.
    pub fn from_shared_bgra(width: u32, height: u32, bytes: Arc<[u8]>) -> Option<Self> {
        if bytes.len() as u64 != u64::from(width) * u64::from(height) * 4 {
            return None;
        }
        let mut image = Self::new(SmallVec::new());
        image.shared = Some(SharedFrame {
            width,
            height,
            bytes,
        });
        Some(image)
    }

    /// Convert this image into a byte slice.
    pub fn as_bytes(&self, frame_index: usize) -> Option<&[u8]> {
        if let Some(shared) = &self.shared {
            return (frame_index == 0).then(|| &*shared.bytes); // KOMOREBI PATCH (shared-pixels)
        }
        self.data
            .get(frame_index)
            .map(|frame| frame.buffer().as_raw().as_slice())
    }

    /// Get the size of this image, in pixels.
    pub fn size(&self, frame_index: usize) -> Size<DevicePixels> {
        if let Some(shared) = &self.shared {
            // KOMOREBI PATCH (shared-pixels)
            return if frame_index == 0 {
                size(shared.width.into(), shared.height.into())
            } else {
                Size::default()
            };
        }
        self.data
            .get(frame_index)
            .map(|frame| {
                let (width, height) = frame.buffer().dimensions();
                size(width.into(), height.into())
            })
            .unwrap_or_default()
    }

    /// Get the size of this image, in pixels for display, adjusted for the scale factor.
    pub(crate) fn render_size(&self, frame_index: usize) -> Size<Pixels> {
        self.size(frame_index)
            .map(|v| (v.0 as f32 / self.scale_factor).into())
    }

    /// Get the delay of this frame from the previous
    pub fn delay(&self, frame_index: usize) -> Delay {
        if self.shared.is_some() && frame_index == 0 {
            // KOMOREBI PATCH (shared-pixels): what `Frame::new` gives a still image.
            return Delay::from_numer_denom_ms(0, 1);
        }
        self.data
            .get(frame_index)
            .map(|frame| frame.delay())
            .unwrap_or(Delay::from_numer_denom_ms(100, 1))
    }

    /// Get the number of frames for this image.
    pub fn frame_count(&self) -> usize {
        self.data.len() + usize::from(self.shared.is_some()) // KOMOREBI PATCH (shared-pixels)
    }
}

impl fmt::Debug for RenderImage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ImageData")
            .field("id", &self.id)
            .field(
                "size",
                &self
                    .data
                    .first()
                    .map(|f| f.buffer().dimensions())
                    .or(self.shared.as_ref().map(|s| (s.width, s.height))), // KOMOREBI PATCH (shared-pixels)
            )
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use smallvec::SmallVec;

    #[test]
    fn empty_render_image_does_not_panic() {
        let image = RenderImage::new(SmallVec::new());
        assert_eq!(image.frame_count(), 0);
        assert_eq!(image.size(0), Size::default());
        assert_eq!(image.as_bytes(0), None);
        assert_eq!(image.render_size(0), Size::default());
        assert_eq!(image.delay(0), Delay::from_numer_denom_ms(100, 1));
        let _ = format!("{image:?}");
    }

    // KOMOREBI PATCH (shared-pixels)
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
        let _ = format!("{image:?}");
    }
}
