//! docgen CLI — the only component allowed to touch the filesystem.

use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use clap::{Parser, Subcommand};
use docgen_core::parse_document;
use docgen_mermaid::MermaidRenderer;
use docgen_typst::TypstRenderer;

#[derive(Parser)]
#[command(name = "docgen", version, about = "Render and diff documentation")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Render a markdown document (with YAML frontmatter) to PDF.
    Render {
        /// Input markdown file.
        input: PathBuf,
        /// Output file; defaults to <input>.pdf.
        #[arg(short, long)]
        out: Option<PathBuf>,
        /// Custom typst template for the main file. It can
        /// `#import "docgen-meta.typ": meta` and must place the body with
        /// `#include "docgen-body.typ"`.
        #[arg(short, long)]
        template: Option<PathBuf>,
    },
    /// Render a standalone mermaid diagram to SVG or PNG.
    Diagram {
        /// Input mermaid source file.
        input: PathBuf,
        /// Output file; defaults to <input>.svg.
        #[arg(short, long)]
        out: Option<PathBuf>,
        /// Output format (svg or png).
        #[arg(short, long, default_value = "svg")]
        format: String,
    },
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Render {
            input,
            out,
            template,
        } => render(input, out, template),
        Command::Diagram {
            input,
            out,
            format,
        } => diagram(input, out, &format),
    }
}

fn render(input: PathBuf, out: Option<PathBuf>, template: Option<PathBuf>) -> anyhow::Result<()> {
    let markdown = std::fs::read_to_string(&input)
        .with_context(|| format!("reading {}", input.display()))?;
    let doc = parse_document(&markdown).context("parsing document")?;

    let outcome = docgen_core::assets::render_diagrams(&doc, &MermaidRenderer::new())
        .context("rendering diagrams")?;
    for skipped in &outcome.skipped {
        eprintln!("warning: no renderer for diagram block {skipped}; emitting placeholder");
    }

    let mut renderer = TypstRenderer::new_embedded();
    if let Some(template_path) = template {
        let template = std::fs::read_to_string(&template_path)
            .with_context(|| format!("reading template {}", template_path.display()))?;
        renderer = renderer.with_template(template);
    }
    let (pdf, warnings) = renderer
        .render_pdf(&doc, &outcome.assets)
        .context("typst compilation")?;
    for w in &warnings {
        eprintln!("warning: {w}");
    }

    let out = out.unwrap_or_else(|| with_extension(&input, "pdf"));
    std::fs::write(&out, pdf).with_context(|| format!("writing {}", out.display()))?;
    println!("wrote {}", out.display());
    Ok(())
}

fn diagram(input: PathBuf, out: Option<PathBuf>, format: &str) -> anyhow::Result<()> {
    let source = std::fs::read_to_string(&input)
        .with_context(|| format!("reading {}", input.display()))?;
    let renderer = MermaidRenderer::new();
    let (bytes, out) = match format.to_ascii_lowercase().as_str() {
        "svg" => {
            let svg = renderer.render_svg(&source).context("mermaid render")?;
            (svg.into_bytes(), out.unwrap_or_else(|| with_extension(&input, "svg")))
        }
        "png" => {
            let png = renderer.render_png(&source).context("mermaid render")?;
            (png, out.unwrap_or_else(|| with_extension(&input, "png")))
        }
        other => bail!("unsupported diagram format {other:?} (use svg or png)"),
    };
    std::fs::write(&out, bytes).with_context(|| format!("writing {}", out.display()))?;
    println!("wrote {}", out.display());
    Ok(())
}

fn with_extension(path: &Path, ext: &str) -> PathBuf {
    path.with_extension(ext)
}
