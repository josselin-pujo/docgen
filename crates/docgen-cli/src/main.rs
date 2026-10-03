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
    /// Render a markdown document (with YAML frontmatter) to PDF or DOCX.
    Render {
        /// Input markdown file.
        input: PathBuf,
        /// Output file; defaults to <input>.<format>.
        #[arg(short, long)]
        out: Option<PathBuf>,
        /// Output format: pdf (typst) or docx (OOXML).
        #[arg(short, long, default_value = "pdf")]
        format: String,
        /// Template. For pdf: a typst template (see docgen-typst docs).
        /// For docx: a .docx template with {{placeholders}} and a
        /// {{docgen:body}} marker paragraph.
        #[arg(short, long)]
        template: Option<PathBuf>,
    },
    /// Diff two revisions of a markdown document into a DOCX with native
    /// Word tracked changes (w:ins/w:del) and comments on moves/diagrams.
    Diff {
        /// The old revision.
        old: PathBuf,
        /// The new revision.
        new: PathBuf,
        /// Output docx; defaults to <new>.diff.docx.
        #[arg(short, long)]
        out: Option<PathBuf>,
        /// Author attributed to the tracked changes.
        #[arg(short, long, default_value = "docgen")]
        author: String,
        /// Revision date (ISO-8601). Defaults to the new document's
        /// frontmatter date, else the epoch — output stays deterministic.
        #[arg(long)]
        date: Option<String>,
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
            format,
            template,
        } => render(input, out, &format, template),
        Command::Diff {
            old,
            new,
            out,
            author,
            date,
        } => diff(old, new, out, &author, date),
        Command::Diagram {
            input,
            out,
            format,
        } => diagram(input, out, &format),
    }
}

fn render(
    input: PathBuf,
    out: Option<PathBuf>,
    format: &str,
    template: Option<PathBuf>,
) -> anyhow::Result<()> {
    let markdown = std::fs::read_to_string(&input)
        .with_context(|| format!("reading {}", input.display()))?;
    let doc = parse_document(&markdown).context("parsing document")?;

    match format.to_ascii_lowercase().as_str() {
        "pdf" => {
            let outcome = docgen_core::assets::render_diagrams(&doc, &MermaidRenderer::new())
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
            let outcome =
                docgen_core::assets::render_diagrams(&doc, &MermaidRenderer::raster())
                    .context("rendering diagrams")?;
            warn_skipped(&outcome.skipped);

            let bytes = match template {
                Some(template_path) => {
                    let template = std::fs::read(&template_path).with_context(|| {
                        format!("reading template {}", template_path.display())
                    })?;
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
        other => bail!("unsupported format {other:?} (use pdf or docx)"),
    }
    Ok(())
}

fn diff(
    old_path: PathBuf,
    new_path: PathBuf,
    out: Option<PathBuf>,
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

    let old_outcome =
        docgen_core::assets::render_diagrams(&old, &MermaidRenderer::raster())
            .context("rendering old diagrams")?;
    let new_outcome =
        docgen_core::assets::render_diagrams(&new, &MermaidRenderer::raster())
            .context("rendering new diagrams")?;

    let identity = match date {
        Some(d) => docgen_office::docx::RevisionIdentity::new(author, d),
        None => docgen_office::docx::RevisionIdentity::from_frontmatter(author, &new.frontmatter),
    };

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
    Ok(())
}

fn warn_skipped(skipped: &[String]) {
    for s in skipped {
        eprintln!("warning: no renderer for diagram block {s}; emitting placeholder");
    }
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
