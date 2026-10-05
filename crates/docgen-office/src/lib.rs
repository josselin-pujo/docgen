//! docgen-office: OOXML output for docgen.
//!
//! DOCX today (M2): fresh rendering and template filling. The diff
//! renderer with **native Word tracked changes** (`w:ins`/`w:del`,
//! revision authors/dates, Word comments on moves/diagram changes) is
//! implemented and tested but **parked**: it has no public surface (CLI,
//! wasm, this crate) until bidirectional updates — how a reviewed DOCX
//! flows back into the markdown source — are designed. See the park note
//! on `docx::render_diff_docx`. PPTX (M3) exposes fresh render, template
//! filling and revision-summary diff decks.
//!
//! Everything is in-memory: templates and outputs are byte buffers. Zip
//! entries carry fixed timestamps so output is byte-stable across runs.

pub mod docx;
pub mod pptx;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("docx generation failed: {0}")]
    Docx(String),

    #[error("pptx generation failed: {0}")]
    Pptx(String),

    #[error("docx template error: {0}")]
    Template(String),

    #[error("zip packaging failed: {0}")]
    Zip(String),

    #[error("XML error: {0}")]
    Xml(String),
}

pub type Result<T> = std::result::Result<T, Error>;
