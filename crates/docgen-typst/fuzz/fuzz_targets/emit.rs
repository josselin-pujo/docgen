#![no_main]

use libfuzzer_sys::fuzz_target;

/// Arbitrary text through the typst emitter (frontmatter → typst dict,
/// inline styles → escaped markup): must never panic, only produce
/// source strings.
fuzz_target!(|data: &[u8]| {
    let markdown = String::from_utf8_lossy(data);
    if let Ok(doc) = docgen_core::parse_document(&markdown) {
        let _ = docgen_typst::emit::emit(&doc, &Default::default(), None);
    }
});
