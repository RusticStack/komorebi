//! The render worker as the GPUI reader uses it (`komorebi::reader`, the
//! same worker as the Slint app's): tiles in BGRA, the order GPUI's
//! `RenderImage` holds, turned into images on the worker thread itself, so
//! the UI thread only places them; the page's lines of text for screen
//! readers; replies handed to the UI thread over a futures channel
//! (cookbook_async). Evicted tiles leave the GPU's sprite atlas through
//! `App::drop_image` (the view does that, after the frame).

use std::sync::Arc;

use futures::channel::mpsc;
use gpui::RenderImage;
use komorebi::reader::{Fetch, Notify, PixelOrder, Pixels, Reply, Worker};

/// A tile as the view draws it.
pub type TileImage = Arc<RenderImage>;

/// What the worker sends the view.
pub enum Incoming {
    /// A tile, already an image.
    Tile {
        generation: u64,
        index: u32,
        image: TileImage,
    },
    /// Anything else.
    Reply(Reply),
}

/// The image of a tile's pixels (BGRA, premultiplied, opaque: a page has
/// its own background).
pub fn image(pixels: &Pixels) -> Option<TileImage> {
    RenderImage::from_shared_bgra(pixels.width, pixels.height, pixels.data.clone()).map(Arc::new)
}

/// A worker for the main reader (or, with `caches`, a message window's),
/// and the channel its replies come in on. The thread starts with the
/// first job and ends when the worker is dropped.
pub fn start(
    caches: Option<(usize, usize)>,
    fetch: Fetch,
) -> (Worker, mpsc::UnboundedReceiver<Incoming>) {
    let (tx, rx) = mpsc::unbounded();
    let notify: Notify = Arc::new(move |reply| {
        let incoming = match reply {
            Reply::Tile {
                generation,
                index,
                pixels,
            } => match image(&pixels) {
                Some(image) => Incoming::Tile {
                    generation,
                    index,
                    image,
                },
                None => return,
            },
            other => Incoming::Reply(other),
        };
        // The view is gone: nothing to tell.
        let _ = tx.unbounded_send(incoming);
    });
    let mut worker = Worker::new(notify, fetch)
        .with_order(PixelOrder::Bgra)
        .with_text_lines();
    if let Some((docs, bytes)) = caches {
        worker = worker.with_caches(docs, bytes);
    }
    (worker, rx)
}

/// Consented remote images come from the network (`komorebi::fetch`,
/// which refuses every request under `KOMOREBI_OFFLINE`).
pub fn network_fetch() -> Fetch {
    Arc::new(|url: &str| komorebi::fetch::get(url).ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tile_becomes_an_image_of_its_size() {
        let data: Vec<u8> = [1, 2, 3, 255].repeat(6);
        let pixels = Pixels::new(3, 2, data.into()).unwrap();
        let img = image(&pixels).expect("an image");
        let size = img.size(0);
        assert_eq!((size.width.0, size.height.0), (3, 2));
        assert_eq!(&img.as_bytes(0).unwrap()[..4], &[1, 2, 3, 255]);
    }
}
