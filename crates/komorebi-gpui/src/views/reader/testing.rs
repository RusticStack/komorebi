//! A fake render worker for the tests of views that make their own readers
//! (the message tabs): `inject` plants one, the next `ReaderView::new` on
//! this thread takes it, and the test plays the worker: it reads the jobs
//! the reader sent and answers with `Incoming` replies. (The reader's own
//! tests build theirs with `ReaderView::with_backend`.)

use std::cell::RefCell;
use std::rc::Rc;

use futures::channel::mpsc;
use komorebi::reader::{Backend, Job, Key, Page, Reply, Shell};

use super::worker::Incoming;

/// Records the jobs.
#[derive(Clone, Default)]
struct Jobs(Rc<RefCell<Vec<Job>>>);

impl Backend for Jobs {
    fn send(&mut self, job: Job) {
        self.0.borrow_mut().push(job);
    }
}

/// Opens nothing.
struct NoShell;

impl Shell for NoShell {
    fn open_url(&self, _: &str) {}
}

type Planted = (
    Box<dyn Backend>,
    Box<dyn Shell>,
    mpsc::UnboundedReceiver<Incoming>,
);

thread_local! {
    static NEXT: RefCell<Option<Planted>> = const { RefCell::new(None) };
}

/// The test's end of a planted worker.
pub(crate) struct Worker {
    jobs: Jobs,
    tx: mpsc::UnboundedSender<Incoming>,
}

/// Plants a fake worker for the next reader made on this thread.
pub(crate) fn inject() -> Worker {
    let jobs = Jobs::default();
    let (tx, rx) = mpsc::unbounded();
    NEXT.with(|next| {
        *next.borrow_mut() = Some((Box::new(jobs.clone()), Box::new(NoShell), rx));
    });
    Worker { jobs, tx }
}

pub(super) fn take() -> Option<Planted> {
    NEXT.with(|next| next.borrow_mut().take())
}

impl Worker {
    /// The layout jobs sent since the last call: generation and key.
    pub(crate) fn layouts(&self) -> Vec<(u64, Key)> {
        std::mem::take(&mut *self.jobs.0.borrow_mut())
            .into_iter()
            .filter_map(|job| match job {
                Job::Layout {
                    generation, key, ..
                } => Some((generation, key)),
                _ => None,
            })
            .collect()
    }

    /// Answers layout `generation` with a page `height` tall that has
    /// `remote_images` blocked images.
    pub(crate) fn page(&self, generation: u64, key: Key, height: f32, remote_images: u32) {
        let page = Page {
            width: key.width as f32,
            height,
            bg: [255; 4],
            truncated: false,
            partial: false,
            remote_images,
            pending_images: false,
            failed_images: 0,
            dimmed: key.dim,
        };
        self.tx
            .unbounded_send(Incoming::Reply(Reply::Page {
                generation,
                page,
                links: vec![],
            }))
            .expect("the reader is listening");
    }
}
