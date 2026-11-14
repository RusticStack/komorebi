//! A counting global allocator for the A/B harness (feature `alloc-count`,
//! off in every shipped build; `scripts/opt-ab.ps1` and docs/PERF.md
//! section 8). It wraps the system allocator and keeps four process-wide
//! counters: allocations (a `realloc` counts as one), bytes asked for, bytes
//! live and the peak of live bytes. `bench.rs` prints them at its marks
//! (`[alloc] <mark> allocs=.. bytes=.. live=.. peak=..`, cumulative).

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering::Relaxed};

static ALLOCS: AtomicU64 = AtomicU64::new(0);
static BYTES: AtomicU64 = AtomicU64::new(0);
static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

/// The counting allocator: `#[global_allocator] static A: Counting = Counting;`.
pub struct Counting;

fn grew(by: usize) {
    ALLOCS.fetch_add(1, Relaxed);
    BYTES.fetch_add(by as u64, Relaxed);
    let live = LIVE.fetch_add(by, Relaxed) + by;
    PEAK.fetch_max(live, Relaxed);
}

// SAFETY: every call is forwarded unchanged to `System`, whose contract the
// caller's does; the counters are atomics with no effect on the memory.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the caller's layout, forwarded.
        let p = unsafe { System.alloc(layout) };
        if !p.is_null() {
            grew(layout.size());
        }
        p
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the caller's layout, forwarded.
        let p = unsafe { System.alloc_zeroed(layout) };
        if !p.is_null() {
            grew(layout.size());
        }
        p
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        LIVE.fetch_sub(layout.size(), Relaxed);
        // SAFETY: the caller's pointer and layout, forwarded.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: the caller's pointer, layout and size, forwarded.
        let p = unsafe { System.realloc(ptr, layout, new_size) };
        if !p.is_null() {
            ALLOCS.fetch_add(1, Relaxed);
            BYTES.fetch_add(new_size as u64, Relaxed);
            LIVE.fetch_sub(layout.size(), Relaxed);
            let live = LIVE.fetch_add(new_size, Relaxed) + new_size;
            PEAK.fetch_max(live, Relaxed);
        }
        p
    }
}

/// The counters now.
pub fn line(mark: &str) -> String {
    format!(
        "[alloc] {mark} allocs={} bytes={} live={} peak={}",
        ALLOCS.load(Relaxed),
        BYTES.load(Relaxed),
        LIVE.load(Relaxed),
        PEAK.load(Relaxed)
    )
}
