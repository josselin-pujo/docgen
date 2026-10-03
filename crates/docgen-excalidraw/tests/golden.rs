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

const SCENE: &str = r##"{"type":"excalidraw","version":2,"elements":[
  {"type":"rectangle","id":"r1","x":10,"y":20,"width":120,"height":60,"strokeColor":"#1e1e1e","backgroundColor":"#a5d8ff","fillStyle":"solid","strokeWidth":2,"roughness":1,"opacity":100,"seed":42,"angle":0,"version":1,"isDeleted":false},
  {"type":"arrow","id":"a1","x":140,"y":50,"width":80,"height":0,"points":[[0,0],[80,0]],"strokeColor":"#1e1e1e","strokeWidth":1,"roughness":1,"opacity":100,"seed":7,"endArrowhead":"arrow","startArrowhead":null,"fillStyle":"hachure","version":1,"isDeleted":false,
   "startBinding":{"elementId":"r1","focus":0,"gap":1,"fixedPoint":null}},
  {"type":"text","id":"t1","x":20,"y":35,"width":100,"height":25,"text":"docgen","fontSize":20,"fontFamily":4,"textAlign":"center","verticalAlign":"middle","containerId":"r1","strokeColor":"#1e1e1e","backgroundColor":"transparent","fillStyle":"hachure","strokeWidth":1,"roughness":1,"opacity":100,"seed":9,"angle":0,"lineHeight":1.25,"version":1,"isDeleted":false}
],"appState":{"viewBackgroundColor":"#ffffff"},"files":{}}"##;

#[test]
fn golden_svg() {
    let svg = docgen_excalidraw::render_source_svg(SCENE).unwrap();
    check("excalidraw-svg", svg.as_bytes());
}
