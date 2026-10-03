//! Golden hashes: outputs must be byte-stable. Regenerate with
//! `UPDATE_GOLDEN=1 cargo test` after an INTENTIONAL output change; a
//! failing hash without one is a regression.

use sha2::{Digest, Sha256};
use std::fs;
use std::path::PathBuf;

fn check(name: &str, bytes: &[u8]) {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden");
    let path = dir.join(format!("{name}.sha256"));
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest: String = hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    if std::env::var("UPDATE_GOLDEN").is_ok() {
        fs::create_dir_all(&dir).unwrap();
        fs::write(&path, &digest).unwrap();
        eprintln!("updated golden {name}");
        return;
    }
    let expected = fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("golden file for {name} missing; run UPDATE_GOLDEN=1"));
    assert_eq!(digest, expected.trim(), "{name} output changed");
}

const DOC: &str = "---\ntitle: Golden\nsubtitle: fixtures\nauthors: [docgen]\n---\n\n# Head {#h}\n\nText with **bold**, _italic_, `code` and [links](https://example.com).\n\n1. first\n2. second\n\n| A | B |\n|---|---|\n| 1 | 2 |\n\n```rust\nfn main() {}\n```\n";

const DOC_V2: &str = "---\ntitle: Golden\nsubtitle: fixtures\nauthors: [docgen]\n---\n\n# Head {#h}\n\nText with **bold**, _italics_, `code` and [links](https://example.com) edited.\n\n1. first\n2. second\n3. inserted\n\n| A | B |\n|---|---|\n| 1 | 2 |\n| 3 | 4 |\n";

#[test]
fn golden_docx() {
    let doc = docgen_core::parse_document(DOC).unwrap();
    let bytes = docgen_office::docx::render_docx(&doc, &Default::default()).unwrap();
    check("docx-fresh", &bytes);
}

#[test]
fn golden_pptx() {
    let doc = docgen_core::parse_document(DOC).unwrap();
    let bytes = docgen_office::pptx::render_pptx(&doc, &Default::default()).unwrap();
    check("pptx-fresh", &bytes);
}

#[test]
fn golden_diff_docx() {
    let old = docgen_core::parse_document(DOC).unwrap();
    let new = docgen_core::parse_document(DOC_V2).unwrap();
    let identity = docgen_office::docx::RevisionIdentity::new("golden", "2026-10-03T00:00:00Z");
    let bytes = docgen_office::docx::render_diff_docx(
        &old,
        &new,
        &Default::default(),
        &Default::default(),
        &identity,
    )
    .unwrap();
    check("docx-diff", &bytes);
}
