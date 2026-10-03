use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("invalid frontmatter: {0}")]
    Frontmatter(String),

    #[error("diagram rendering failed: {0}")]
    Diagram(String),

    #[error("layout/rendering backend failed: {0}")]
    Backend(String),
}

pub type Result<T> = std::result::Result<T, Error>;
