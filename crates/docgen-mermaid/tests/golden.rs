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

const SRC: &str = "flowchart LR
    MD[Markdown] --> Core[docgen-core]
    Core -->|render| Out[PDF]
";

#[test]
fn golden_svg() {
    let svg = docgen_mermaid::MermaidRenderer::new()
        .render_svg(SRC)
        .unwrap();
    check("mermaid-svg", svg.as_bytes());
}
