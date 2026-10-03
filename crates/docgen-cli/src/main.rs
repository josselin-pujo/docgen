//! docgen CLI — the only component allowed to touch the filesystem.

use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use clap::{Parser, Subcommand};
use docgen_core::parse_document;
use docgen_excalidraw::ExcalidrawRenderer;
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
    /// Render a markdown document (with YAML frontmatter) to PDF, DOCX or
    /// PPTX.
    Render {
        /// Input markdown file.
        input: PathBuf,
        /// Output file; defaults to <input>.<format>.
        #[arg(short, long)]
        out: Option<PathBuf>,
        /// Output format: pdf (typst), docx (OOXML) or pptx
        /// (PresentationML).
        #[arg(short, long, default_value = "pdf")]
        format: String,
        /// Template. For pdf: a typst template (see docgen-typst docs).
        /// For docx: a .docx template with {{placeholders}} and a
        /// {{docgen:body}} marker paragraph. For pptx: a .pptx whose
        /// masters/layouts/theme provide the look; generated slides are
        /// appended.
        #[arg(short, long)]
        template: Option<PathBuf>,
    },
    /// Diff two revisions of a markdown document into a DOCX with native
    /// Word tracked changes (w:ins/w:del) and comments on moves/diagrams,
    /// or a PPTX with revision chips and a summary slide.
    Diff {
        /// The old revision.
        old: PathBuf,
        /// The new revision.
        new: PathBuf,
        /// Output file; defaults to <new>.diff.docx / <new>.diff.pptx.
        #[arg(short, long)]
        out: Option<PathBuf>,
        /// Output format (docx or pptx).
        #[arg(short, long, default_value = "docx")]
        format: String,
        /// Author attributed to the tracked changes.
        #[arg(short, long, default_value = "docgen")]
        author: String,
        /// Revision date (ISO-8601). Defaults to the new document's
        /// frontmatter date, else the epoch — output stays deterministic.
        #[arg(long)]
        date: Option<String>,
    },
    /// Render a standalone diagram (mermaid or raw excalidraw JSON) to
    /// SVG or PNG.
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
            format,
            template,
        } => render(input, out, &format, template),
        Command::Diff {
            old,
            new,
            out,
            format,
            author,
            date,
        } => diff(old, new, out, &format, &author, date),
        Command::Diagram { input, out, format } => diagram(input, out, &format),
    }
}

fn render(
    input: PathBuf,
    out: Option<PathBuf>,
    format: &str,
    template: Option<PathBuf>,
) -> anyhow::Result<()> {
    let markdown =
        std::fs::read_to_string(&input).with_context(|| format!("reading {}", input.display()))?;
    let doc = parse_document(&markdown).context("parsing document")?;

    match format.to_ascii_lowercase().as_str() {
        "pdf" => {
            let excalidraw = ExcalidrawRenderer::new();
            let mermaid = MermaidRenderer::new();
            let outcome = docgen_core::assets::render_diagrams_with(&doc, &[&mermaid, &excalidraw])
                .context("rendering diagrams")?;
            warn_skipped(&outcome.skipped);

            let mut renderer = TypstRenderer::new_embedded();
            if let Some(template_path) = template {
                let tpl = std::fs::read_to_string(&template_path)
                    .with_context(|| format!("reading template {}", template_path.display()))?;
                renderer = renderer.with_template(tpl);
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
        }
        "docx" => {
            // Diagrams become PNG images inside the docx.
            let excalidraw = ExcalidrawRenderer::raster();
            let mermaid = MermaidRenderer::raster();
            let outcome = docgen_core::assets::render_diagrams_with(&doc, &[&mermaid, &excalidraw])
                .context("rendering diagrams")?;
            warn_skipped(&outcome.skipped);

            let bytes = match template {
                Some(template_path) => {
                    let template = std::fs::read(&template_path)
                        .with_context(|| format!("reading template {}", template_path.display()))?;
                    docgen_office::docx::fill_template(&template, &doc, &outcome.assets)
                        .context("filling docx template")?
                }
                None => docgen_office::docx::render_docx(&doc, &outcome.assets)
                    .context("rendering docx")?,
            };
            docgen_office::docx::validate_docx(&bytes).context("validating docx")?;
            let out = out.unwrap_or_else(|| with_extension(&input, "docx"));
            std::fs::write(&out, bytes).with_context(|| format!("writing {}", out.display()))?;
            println!("wrote {}", out.display());
        }
        "pptx" => {
            // Diagrams become PNG images inside the deck.
            let excalidraw = ExcalidrawRenderer::raster();
            let mermaid = MermaidRenderer::raster();
            let outcome = docgen_core::assets::render_diagrams_with(&doc, &[&mermaid, &excalidraw])
                .context("rendering diagrams")?;
            warn_skipped(&outcome.skipped);

            let bytes = match template {
                Some(template_path) => {
                    let template = std::fs::read(&template_path)
                        .with_context(|| format!("reading template {}", template_path.display()))?;
                    docgen_office::pptx::fill_template(&template, &doc, &outcome.assets)
                        .context("filling pptx template")?
                }
                None => docgen_office::pptx::render_pptx(&doc, &outcome.assets)
                    .context("rendering pptx")?,
            };
            docgen_office::pptx::validate_pptx(&bytes).context("validating pptx")?;
            let out = out.unwrap_or_else(|| with_extension(&input, "pptx"));
            std::fs::write(&out, bytes).with_context(|| format!("writing {}", out.display()))?;
            println!("wrote {}", out.display());
        }
        other => bail!("unsupported format {other:?} (use pdf, docx or pptx)"),
    }
    Ok(())
}

fn diff(
    old_path: PathBuf,
    new_path: PathBuf,
    out: Option<PathBuf>,
    format: &str,
    author: &str,
    date: Option<String>,
) -> anyhow::Result<()> {
    let old_md = std::fs::read_to_string(&old_path)
        .with_context(|| format!("reading {}", old_path.display()))?;
    let new_md = std::fs::read_to_string(&new_path)
        .with_context(|| format!("reading {}", new_path.display()))?;
    let old = parse_document(&old_md).context("parsing old revision")?;
    let new = parse_document(&new_md).context("parsing new revision")?;

    let report = docgen_core::diff::diff_documents(&old, &new);
    println!(
        "{} → {}: {}",
        old_path.display(),
        new_path.display(),
        report.summary()
    );

    let excalidraw = ExcalidrawRenderer::raster();
    let mermaid = MermaidRenderer::raster();
    let old_outcome = docgen_core::assets::render_diagrams_with(&old, &[&mermaid, &excalidraw])
        .context("rendering old diagrams")?;
    let new_outcome = docgen_core::assets::render_diagrams_with(&new, &[&mermaid, &excalidraw])
        .context("rendering new diagrams")?;

    let identity = match date {
        Some(d) => docgen_office::docx::RevisionIdentity::new(author, d),
        None => docgen_office::docx::RevisionIdentity::from_frontmatter(author, &new.frontmatter),
    };

    match format.to_ascii_lowercase().as_str() {
        "docx" => {
            let bytes = docgen_office::docx::render_diff_docx(
                &old,
                &new,
                &old_outcome.assets,
                &new_outcome.assets,
                &identity,
            )
            .context("rendering diff docx")?;
            docgen_office::docx::validate_docx(&bytes).context("validating docx")?;
            let out = out.unwrap_or_else(|| with_extension(&new_path, "diff.docx"));
            std::fs::write(&out, bytes).with_context(|| format!("writing {}", out.display()))?;
            println!("wrote {}", out.display());
        }
        "pptx" => {
            let bytes = docgen_office::pptx::render_diff_pptx(
                &old,
                &new,
                &old_outcome.assets,
                &new_outcome.assets,
                &identity,
            )
            .context("rendering diff pptx")?;
            docgen_office::pptx::validate_pptx(&bytes).context("validating pptx")?;
            let out = out.unwrap_or_else(|| with_extension(&new_path, "diff.pptx"));
            std::fs::write(&out, bytes).with_context(|| format!("writing {}", out.display()))?;
            println!("wrote {}", out.display());
        }
        other => bail!("unsupported diff format {other:?} (use docx or pptx)"),
    }
    Ok(())
}

fn warn_skipped(skipped: &[String]) {
    for s in skipped {
        eprintln!("warning: no renderer for diagram block {s}; emitting placeholder");
    }
}

/// A standalone diagram source: mermaid text or raw `.excalidraw` /
/// excalidraw JSON (detected by extension, falling back to content
/// sniffing — an object with an `elements` array).
enum DiagramSource {
    Mermaid(String),
    Excalidraw(String),
}

fn detect_diagram(input: &Path, source: String) -> DiagramSource {
    let ext = input
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default();
    if ext == "excalidraw" || ext == "json" {
        return DiagramSource::Excalidraw(source);
    }
    let trimmed = source.trim_start();
    if trimmed.starts_with('{')
        && (source.contains("\"elements\"") || source.contains("\"type\": \"excalidraw\""))
    {
        return DiagramSource::Excalidraw(source);
    }
    DiagramSource::Mermaid(source)
}

fn diagram(input: PathBuf, out: Option<PathBuf>, format: &str) -> anyhow::Result<()> {
    let source =
        std::fs::read_to_string(&input).with_context(|| format!("reading {}", input.display()))?;
    let (bytes, out) = match (
        detect_diagram(&input, source),
        format.to_ascii_lowercase().as_str(),
    ) {
        (DiagramSource::Mermaid(source), "svg") => {
            let svg = MermaidRenderer::new()
                .render_svg(&source)
                .context("mermaid render")?;
            (
                svg.into_bytes(),
                out.unwrap_or_else(|| with_extension(&input, "svg")),
            )
        }
        (DiagramSource::Mermaid(source), "png") => {
            let png = MermaidRenderer::new()
                .render_png(&source)
                .context("mermaid render")?;
            (png, out.unwrap_or_else(|| with_extension(&input, "png")))
        }
        (DiagramSource::Excalidraw(source), "svg") => {
            let svg = docgen_excalidraw::render_source_svg(&source).context("excalidraw render")?;
            (
                svg.into_bytes(),
                out.unwrap_or_else(|| with_extension(&input, "svg")),
            )
        }
        (DiagramSource::Excalidraw(source), "png") => {
            let png = docgen_excalidraw::render_source_png(&source).context("excalidraw render")?;
            (png, out.unwrap_or_else(|| with_extension(&input, "png")))
        }
        (_, other) => bail!("unsupported diagram format {other:?} (use svg or png)"),
    };
    std::fs::write(&out, bytes).with_context(|| format!("writing {}", out.display()))?;
    println!("wrote {}", out.display());
    Ok(())
}

fn with_extension(path: &Path, ext: &str) -> PathBuf {
    path.with_extension(ext)
}
