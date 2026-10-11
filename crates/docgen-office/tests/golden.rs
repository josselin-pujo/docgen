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

#[test]
fn golden_docx() {
    let doc = docgen_core::parse_document(DOC).unwrap();
    let bytes = docgen_office::docx::render_docx(&doc, &Default::default()).unwrap();
    check("docx-fresh", &bytes);
}

const DOC_TOC: &str = "---\ntitle: Golden TOC\nsubtitle: fixtures\nauthors: [docgen]\ntoc: true\n---\n\n# Head {#h}\n\nText with **bold**.\n\n## Sub {#s}\n\nMore text.\n\n### Deep {#d}\n\nDeep text.\n";

#[test]
fn golden_docx_toc() {
    let doc = docgen_core::parse_document(DOC_TOC).unwrap();
    let bytes = docgen_office::docx::render_docx(&doc, &Default::default()).unwrap();
    check("docx-toc", &bytes);
}

/// Minimal valid 1x1 white PNG (decodes in real decoders).
const TINY_PNG: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4,
    0x89, 0x00, 0x00, 0x00, 0x0B, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0xF8, 0x0F, 0x04, 0x00,
    0x09, 0xFB, 0x03, 0xFD, 0xFB, 0x5E, 0x6B, 0x2B, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44,
    0xAE, 0x42, 0x60, 0x82,
];

const DOC_IMAGE: &str =
    "---\ntitle: Golden Image\n---\n\n# Head {#h}\n\nBefore ![pixel](img/pixel.png) after.\n";

#[test]
fn golden_docx_image() {
    let doc = docgen_core::parse_document(DOC_IMAGE).unwrap();
    let mut assets = std::collections::BTreeMap::new();
    assets.insert(
        "img/pixel.png".to_string(),
        docgen_core::assets::RenderedAsset {
            format: docgen_core::assets::AssetFormat::Png,
            bytes: TINY_PNG.to_vec(),
        },
    );
    let bytes = docgen_office::docx::render_docx(&doc, &assets).unwrap();
    check("docx-image", &bytes);
}

#[test]
fn golden_pptx() {
    let doc = docgen_core::parse_document(DOC).unwrap();
    let bytes = docgen_office::pptx::render_pptx(&doc, &Default::default()).unwrap();
    check("pptx-fresh", &bytes);
}
