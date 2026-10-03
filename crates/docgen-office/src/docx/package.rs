//! Deterministic in-memory zip packaging: fixed entry order and a fixed
//! 1980-01-01 timestamp so identical input yields identical bytes.

use std::io::Write;

use crate::{Error, Result};

pub struct Package {
    entries: Vec<(String, Vec<u8>)>,
}

impl Default for Package {
    fn default() -> Self {
        Self::new()
    }
}

impl Package {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    pub fn add(&mut self, name: impl Into<String>, bytes: Vec<u8>) {
        self.entries.push((name.into(), bytes));
    }

    pub fn build(self) -> Result<Vec<u8>> {
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let timestamp = zip::DateTime::from_date_and_time(1980, 1, 1, 0, 0, 0)
            .map_err(|e| Error::Zip(format!("fixed timestamp rejected: {e}")))?;
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .last_modified_time(timestamp);
        for (name, bytes) in &self.entries {
            zip.start_file(name.as_str(), options)
                .map_err(|e| Error::Zip(format!("starting {name}: {e}")))?;
            zip.write_all(bytes)
                .map_err(|e| Error::Zip(format!("writing {name}: {e}")))?;
        }
        let cursor = zip
            .finish()
            .map_err(|e| Error::Zip(format!("finishing archive: {e}")))?;
        Ok(cursor.into_inner())
    }
}

/// Parses the width/height of a PNG from its IHDR chunk.
pub fn png_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 24 || !bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        return None;
    }
    let width = u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
    let height = u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
    Some((width, height))
}
