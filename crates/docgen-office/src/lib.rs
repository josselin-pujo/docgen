//! docgen-office: OOXML output for docgen.
//!
//! DOCX today (M2): fresh rendering, diff rendering with **native Word
//! tracked changes** (`w:ins`/`w:del`, revision authors/dates) plus Word
//! comments anchored to moved/changed blocks, and template filling.
//! PPTX is M3.
//!
//! Everything is in-memory: templates and outputs are byte buffers. Zip
//! entries carry fixed timestamps so output is byte-stable across runs.

pub mod docx;
pub mod pptx;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("docx generation failed: {0}")]
    Docx(String),

    #[error("docx template error: {0}")]
    Template(String),

    #[error("zip packaging failed: {0}")]
    Zip(String),

    #[error("XML error: {0}")]
    Xml(String),
}

pub type Result<T> = std::result::Result<T, Error>;
