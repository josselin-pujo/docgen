//! Renders every page of a markdown document to PNG files.
//!
//! Usage: cargo run -p docgen-cli --example render-pages -- input.md outdir [scale] [template.typ]

use std::path::PathBuf;

use docgen_core::assets::render_diagrams;
use docgen_core::parse_document;
use docgen_mermaid::MermaidRenderer;
use docgen_typst::TypstRenderer;

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let input = PathBuf::from(args.next().expect("input markdown path"));
    let outdir = PathBuf::from(args.next().expect("output directory"));
    let scale: f64 = args.next().and_then(|s| s.parse().ok()).unwrap_or(2.0);
    let template_path = args.next().map(PathBuf::from);

    let markdown = std::fs::read_to_string(&input)?;
    let doc = parse_document(&markdown)?;
    let outcome = render_diagrams(&doc, &MermaidRenderer::new())?;
    std::fs::create_dir_all(&outdir)?;

    let mut renderer = TypstRenderer::new_embedded();
    if let Some(path) = template_path {
        let template = std::fs::read_to_string(&path)?;
        renderer = renderer.with_template(template);
    }
    let (pages, warnings) = renderer.render_png_pages(&doc, &outcome.assets, scale)?;
    for w in &warnings {
        eprintln!("warning: {w}");
    }
    for (i, png) in pages.iter().enumerate() {
        let path = outdir.join(format!("page-{:03}.png", i + 1));
        std::fs::write(&path, png)?;
        println!("wrote {}", path.display());
    }
    Ok(())
}
