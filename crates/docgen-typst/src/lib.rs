//! PDF rendering for docgen documents via an embedded typst compiler.
//!
//! The whole pipeline is in-memory: the document model is lowered to typst
//! markup (`docgen-body.typ`), frontmatter is exposed as a typst dictionary
//! (`docgen-meta.typ`), and compilation runs against a custom
//! [`typst::World`] backed by `String`/`Vec<u8>` buffers. The main file is a
//! template — the built-in default, or any custom typst source provided via
//! [`TypstRenderer::with_template`] that imports/includes those two modules.
//!
//! Fonts come from `typst-assets` (Libertinus + NewCM math + DejaVu mono),
//! so output is deterministic and needs no system font discovery.
//! `World::today` deliberately returns `None`: no clocks in the library, ever.
//! Document dates should come from frontmatter.

mod emit;
mod world;

use docgen_core::assets::AssetMap;
use docgen_core::model::Document;
use docgen_core::{Error, Result};
use typst::diag::{SourceDiagnostic, Warned};
use typst::text::{Font, FontBook};
use typst::utils::LazyHash;
use typst::{Library, LibraryExt};
use typst_layout::PagedDocument;
use typst_pdf::PdfOptions;
use world::MemWorld;

pub use emit::{BODY_PATH, META_PATH};

pub struct TypstRenderer {
    library: LazyHash<Library>,
    fonts: Vec<Font>,
    book: LazyHash<FontBook>,
    template: Option<String>,
}

impl Default for TypstRenderer {
    fn default() -> Self {
        Self::new_embedded()
    }
}

impl TypstRenderer {
    /// Renderer with the typst-assets font set compiled into the binary and
    /// the built-in document template.
    pub fn new_embedded() -> Self {
        let fonts: Vec<Font> = typst_assets::fonts()
            .filter_map(|data| Font::new(typst::foundations::Bytes::new(data), 0))
            .collect();
        let book = FontBook::from_fonts(fonts.iter());
        Self {
            library: LazyHash::new(Library::builder().build()),
            book: LazyHash::new(book),
            fonts,
            template: None,
        }
    }

    /// Sets a custom typst template used as the main file. The template can
    /// `#import "docgen-meta.typ": meta` and must place the body with
    /// `#include "docgen-body.typ"`.
    pub fn with_template(mut self, template: impl Into<String>) -> Self {
        self.template = Some(template.into());
        self
    }

    /// The active template, if custom.
    pub fn template(&self) -> Option<&str> {
        self.template.as_deref()
    }

    /// Renders a document to PDF bytes. Returns the PDF plus typst warnings
    /// (non-fatal diagnostics), for the host to surface.
    pub fn render_pdf(&self, doc: &Document, assets: &AssetMap) -> Result<(Vec<u8>, Vec<String>)> {
        let paged = self.compile(doc, assets)?;
        let bytes = typst_pdf::pdf(&paged.0, &PdfOptions::default()).map_err(|errs| {
            let messages: Vec<String> =
                errs.iter().map(|e| e.message.to_string()).collect();
            Error::Backend(messages.join("; "))
        })?;
        Ok((bytes, paged.1))
    }

    /// Renders each page to PNG bytes at `pixel_per_pt` device scale.
    pub fn render_png_pages(
        &self,
        doc: &Document,
        assets: &AssetMap,
        pixel_per_pt: f64,
    ) -> Result<(Vec<Vec<u8>>, Vec<String>)> {
        let (paged, warnings) = self.compile(doc, assets)?;
        let opts = typst_render::RenderOptions {
            pixel_per_pt: typst::utils::Scalar::new(pixel_per_pt),
            render_bleed: false,
        };
        let pages = paged
            .pages()
            .iter()
            .map(|page| {
                typst_render::render(page, &opts)
                    .encode_png()
                    .map_err(|e| Error::Backend(format!("png encoding failed: {e}")))
            })
            .collect::<Result<Vec<_>>>()?;
        Ok((pages, warnings))
    }

    fn compile(&self, doc: &Document, assets: &AssetMap) -> Result<(PagedDocument, Vec<String>)> {
        let emitted = emit::emit(doc, assets, self.template.as_deref());
        let world = MemWorld::new(self, emitted.main, emitted.sources, emitted.files);

        let Warned { output, warnings } = typst::compile::<PagedDocument>(&world);
        let warnings = warnings
            .iter()
            .map(|w| format!("typst warning: {}", w.message))
            .collect();
        let paged = output.map_err(|errs| {
            let messages: Vec<String> = errs
                .iter()
                .map(|e| format!("{} ({})", e.message, world.locate(e.span)))
                .collect();
            Error::Backend(messages.join("; "))
        })?;        Ok((paged, warnings))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use docgen_core::parse_document;

    #[test]
    fn smoke_pdf() {
        let doc = parse_document("# Hello\n\nWorld with **bold** text.\n").unwrap();
        let renderer = TypstRenderer::new_embedded();
        let (bytes, _) = renderer.render_pdf(&doc, &AssetMap::default()).unwrap();
        assert!(bytes.starts_with(b"%PDF"), "not a PDF: {}", bytes.len());
        assert!(bytes.len() > 1_000, "suspiciously small PDF");
    }

    #[test]
    fn smoke_png_pages() {
        let doc = parse_document("# Hello\n\nWorld.\n").unwrap();
        let renderer = TypstRenderer::new_embedded();
        let (pages, _) = renderer
            .render_png_pages(&doc, &AssetMap::default(), 2.0)
            .unwrap();
        assert_eq!(pages.len(), 1);
        assert!(pages[0].starts_with(&[0x89, b'P', b'N', b'G']), "not a PNG");
    }

    #[test]
    fn custom_template_replaces_layout() {
        // 10cm-wide pages instead of A4 → the PNG width at 1.0 px/pt proves
        // the template governed the page setup.
        let doc = parse_document("# Hello\n\nWorld.\n").unwrap();
        let renderer = TypstRenderer::new_embedded().with_template(concat!(
            "#set page(width: 10cm, height: 4cm)\n",
            "#include \"docgen-body.typ\"\n",
        ));
        let (pages, _) = renderer
            .render_png_pages(&doc, &AssetMap::default(), 1.0)
            .unwrap();
        let width = png_width(&pages[0]);
        // 10cm = 283.46pt → 283px at 1.0 px/pt.
        assert!(
            (280..=287).contains(&width),
            "template did not set page width: {width}px"
        );
    }

    #[test]
    fn meta_reaches_template() {
        // Referencing an `extra` frontmatter key compiles only if the key
        // actually made it into docgen-meta.typ.
        let doc = parse_document("---\nanswer: 42\n---\n\nBody.\n").unwrap();
        let renderer = TypstRenderer::new_embedded().with_template(concat!(
            "#import \"docgen-meta.typ\": meta\n",
            "#set page(width: 6cm, height: 3cm)\n",
            "#text[#str(meta.answer)]\n",
            "#include \"docgen-body.typ\"\n",
        ));
        let (bytes, _) = renderer.render_pdf(&doc, &AssetMap::default()).unwrap();
        assert!(bytes.starts_with(b"%PDF"));
    }

    /// PNG IHDR width, big-endian u32 at byte offset 16.
    fn png_width(png: &[u8]) -> u32 {
        u32::from_be_bytes([png[16], png[17], png[18], png[19]])
    }
}
