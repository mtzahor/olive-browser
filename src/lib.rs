//! Olive Browser's inert HTML document parser.
//!
//! Parsing follows html5ever's implementation of the WHATWG HTML Living Standard.
//! HTML parsing builds an inert DOM. The optional `js` module explicitly executes
//! JavaScript; the optional GUI renders pages. The optional `net` module loads
//! HTTP(S) documents only when explicitly called. Parsing itself never fetches URLs.
//!
//! ```
//! use olive_html::{parse, NodeKind};
//!
//! let parsed = parse("<!doctype html><title>Olive</title><p>Hello &amp; welcome!")?;
//! let document = &parsed.document;
//! assert!(document.descendants(document.root()).any(|id| {
//!     matches!(document.node(id).map(|n| &n.kind),
//!         Some(NodeKind::Text(text)) if text == "Hello & welcome!")
//! }));
//! # Ok::<(), olive_html::ParseError>(())
//! ```

#[cfg(feature = "css")]
pub mod css;

#[cfg(feature = "js")]
pub mod js;

#[cfg(feature = "net")]
pub mod net;

mod dom;
mod parser;
mod sink;

pub use dom::{Children, Descendants, Document, Element, Node, NodeId, NodeKind};
pub use html5ever::interface::QuirksMode;
pub use html5ever::{Attribute, QualName};
pub use parser::{
    Diagnostic, ParseError, ParseOptions, ParseOutput, parse, parse_reader, parse_utf8,
};
