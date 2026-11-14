//! A second layout of a message that reuses the first one's parsed
//! document and shaped text ([`render::layout_reusing`]) must come out
//! exactly as a layout from scratch: same pixels, text and geometry.

mod common;

use common::{big_stack, doc_hashes, open_corpus};
use komorebi::mail::render::{self, Deadline, Engine, NoRemote, Opts, Reuse};

fn opts(width_px: f32) -> Opts {
    Opts {
        width_px,
        ..Default::default()
    }
}

#[test]
fn reusing_the_first_layout_changes_nothing() {
    big_stack(|| {
        let mut engine = Engine::new();
        let mut reused = 0usize;
        for (name, input) in open_corpus() {
            // The widths a reader meets: the column, then the natural width
            // of content that cannot shrink to it, then back.
            let mut reuse = Reuse::new(&input);
            let none = Deadline::none();
            let Ok(first) = render::layout_reusing(
                &mut reuse,
                &opts(600.0),
                &mut engine,
                &none,
                &mut NoRemote,
                None,
            ) else {
                continue;
            };
            for width in [708.0, 420.0] {
                let fresh = render::layout(&input, &opts(width), &mut engine, &none).unwrap();
                let again = render::layout_reusing(
                    &mut reuse,
                    &opts(width),
                    &mut engine,
                    &none,
                    &mut NoRemote,
                    Some(&first),
                )
                .unwrap();
                assert_eq!(
                    doc_hashes(&fresh, &mut engine, false),
                    doc_hashes(&again, &mut engine, false),
                    "{name} at {width}"
                );
                reused += again.reused;
            }
        }
        // The corpus has plenty of plain text groups to hand over.
        assert!(reused > 1000, "only {reused} groups reused");
    });
}
