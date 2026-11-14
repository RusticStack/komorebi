//! KOMOREBI PATCH: the text formats without the `text-formats` feature.
//!
//! `TextView` and `TextViewState` parse Markdown and HTML through
//! `format::markdown::parse` and `format::html::parse`. Without the
//! `text-formats` feature neither parser is built (they pull in `markdown`'s
//! parser, `html5ever` and `markup5ever_rcdom`), and these stand in: each
//! reports an error, which a `TextView` shows as an empty document. An
//! application that never renders Markdown or HTML (Komorebi) never calls them.

use gpui::SharedString;

use crate::text::{document::ParsedDocument, node::NodeContext};

const DISABLED: &str = "gpui-base was built without the `text-formats` feature";

pub(super) mod markdown {
    use super::*;

    pub(crate) fn parse(
        _source: &str,
        _cx: &mut NodeContext,
    ) -> Result<ParsedDocument, SharedString> {
        Err(SharedString::from(DISABLED))
    }
}

pub(super) mod html {
    use super::*;

    pub(crate) fn parse(
        _source: &str,
        _cx: &mut NodeContext,
    ) -> Result<ParsedDocument, SharedString> {
        Err(SharedString::from(DISABLED))
    }
}
