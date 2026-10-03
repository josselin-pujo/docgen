//! docgen-core: the in-memory document model for the docgen toolchain.
//!
//! Hard constraints of this crate (and every library crate in the workspace):
//! - no filesystem access, no network access, no clocks, no environment;
//! - inputs are `&str` / `&[u8]`, outputs are owned in-memory values;
//! - must compile for wasm targets (browser-flavored `wasm32-unknown-unknown`
//!   and WASI flavors) — anything that cannot (e.g. JS engines) lives behind
//!   a trait in a downstream crate.
//!
//! I/O belongs to `docgen-cli` (native) or a wasm bindgen shell, never here.

pub mod assets;
pub mod diff;
mod error;
pub mod frontmatter;
pub mod ids;
pub mod model;
pub mod parse;

pub use error::{Error, Result};
pub use model::{Block, BlockKind, DiagramFormat, Document, Frontmatter, Inline, ListItem};
pub use parse::parse_document;
