#![no_main]

use libfuzzer_sys::fuzz_target;

/// Arbitrary bytes through the frontmatter splitter + full parser: any
/// input must parse to a document or a clean error, never panic.
fuzz_target!(|data: &[u8]| {
    let markdown = String::from_utf8_lossy(data);
    let _ = docgen_core::parse_document(&markdown);
});
