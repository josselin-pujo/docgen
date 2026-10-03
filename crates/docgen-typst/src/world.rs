//! The in-memory [`World`] implementation: everything typst can see lives in
//! buffers owned by this struct. No filesystem, no network, no clock.

use std::collections::HashMap;

use typst::diag::{FileError, FileResult};
use typst::foundations::{Bytes, Datetime, Duration};
use typst::syntax::{FileId, RootedPath, Source, VirtualPath, VirtualRoot};
use typst::text::{Font, FontBook};
use typst::utils::LazyHash;
use typst::{Library, World};

use super::TypstRenderer;

pub(crate) struct MemWorld {
    library: LazyHash<Library>,
    book: LazyHash<FontBook>,
    fonts: Vec<Font>,
    main: FileId,
    source: Source,
    main_bytes: Bytes,
    sources: HashMap<FileId, Source>,
    source_bytes: HashMap<FileId, Bytes>,
    files: HashMap<FileId, Bytes>,
    names: HashMap<FileId, String>,
}

impl MemWorld {
    /// `main` is the template source; `sources` are additional typst sources
    /// addressable by relative path (body, meta); `files` are binary virtual
    /// files (rendered diagrams).
    pub(crate) fn new(
        renderer: &TypstRenderer,
        main: String,
        sources: Vec<(String, String)>,
        files: Vec<(String, Vec<u8>)>,
    ) -> Self {
        let main_id = file_id("main.typ").expect("static valid path");
        let main_bytes = Bytes::from_string(main.clone());
        let mut source_map = HashMap::new();
        let mut source_bytes = HashMap::new();
        let mut names = HashMap::new();
        names.insert(main_id, "main.typ".to_string());
        for (path, text) in &sources {
            let id = file_id(path).expect("generated paths are valid");
            source_bytes.insert(id, Bytes::from_string(text.clone()));
            // Sources must carry their real id: typst's cyclic-import guard
            // consults `source.id()` against the evaluation route.
            source_map.insert(id, Source::new(id, text.clone()));
            names.insert(id, path.clone());
        }
        let mut file_map = HashMap::new();
        for (path, bytes) in files {
            let id = file_id(&path).expect("generated paths are valid");
            file_map.insert(id, Bytes::new(bytes));
        }
        Self {
            library: renderer.library.clone(),
            book: renderer.book.clone(),
            fonts: renderer.fonts.clone(),
            main: main_id,
            source: Source::new(main_id, main),
            main_bytes,
            sources: source_map,
            source_bytes,
            files: file_map,
            names,
        }
    }
}

impl World for MemWorld {
    fn library(&self) -> &LazyHash<Library> {
        &self.library
    }

    fn book(&self) -> &LazyHash<FontBook> {
        &self.book
    }

    fn main(&self) -> FileId {
        self.main
    }

    fn source(&self, id: FileId) -> FileResult<Source> {
        if id == self.main {
            Ok(self.source.clone())
        } else {
            self.sources
                .get(&id)
                .cloned()
                .ok_or_else(|| FileError::NotFound(path_of(id)))
        }
    }

    fn file(&self, id: FileId) -> FileResult<Bytes> {
        if id == self.main {
            return Ok(self.main_bytes.clone());
        }
        if let Some(bytes) = self.source_bytes.get(&id).or_else(|| self.files.get(&id)) {
            return Ok(bytes.clone());
        }
        Err(FileError::NotFound(path_of(id)))
    }

    fn font(&self, index: usize) -> Option<Font> {
        self.fonts.get(index).cloned()
    }

    fn today(&self, _offset: Option<Duration>) -> Option<Datetime> {
        // Deterministic output: dates come from frontmatter, never the clock.
        None
    }
}

fn file_id(rel: &str) -> Option<FileId> {
    let vpath = VirtualPath::new(rel).ok()?;
    Some(FileId::new(RootedPath::new(VirtualRoot::Project, vpath)))
}

fn path_of(id: FileId) -> std::path::PathBuf {
    std::path::PathBuf::from(id.get().vpath().get_with_slash().to_string())
}

impl MemWorld {
    /// Human-readable location (file + line) for a diagnostic span, so
    /// template authors get actionable errors.
    pub(crate) fn locate(&self, span: typst::syntax::DiagSpan) -> String {
        use typst::syntax::DiagSpanKind;
        use typst::WorldExt;

        let (id, range) = match span.get() {
            DiagSpanKind::Number { id, .. } => (Some(id), self.range(span)),
            DiagSpanKind::Range { id, range } => (Some(id), Some(range)),
            DiagSpanKind::Detached => (None, None),
        };
        let (Some(id), Some(range)) = (id, range) else {
            return "unknown location".to_string();
        };
        let name = match id {
            i if i == self.main => "main.typ".to_string(),
            i => self
                .names
                .get(&i)
                .cloned()
                .unwrap_or_else(|| "unknown file".to_string()),
        };
        let text = if id == self.main {
            Some(self.source.text().to_string())
        } else {
            self.sources.get(&id).map(|s| s.text().to_string())
        };
        let line = text
            .map(|t| t[..range.start.min(t.len())].matches('\n').count() + 1)
            .unwrap_or(0);
        format!("{name}:{line}")
    }
}
