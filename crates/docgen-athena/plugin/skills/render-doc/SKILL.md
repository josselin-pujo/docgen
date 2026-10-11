---
name: render-doc
description: Render markdown to PDF/DOCX/PPTX and mermaid/excalidraw diagrams to SVG/PNG, fully in-process — blob refs, no shell, no filesystem required.
---

# render-doc

`docgen_compose` renders documents and diagrams inside this session — a
wasm component runs the real docgen pipeline (the same markdown, YAML
frontmatter, and diagram support as the `docgen` CLI, without its diff
features). No executables, no shell. The tool is blob-native: refs in,
refs out — it works the same in sessions with or without a workspace
filesystem.

## Call

```
docgen_compose({
  action: "render" | "diagram",
  format: "pdf" | "docx" | "pptx",   // action: render
  // or   "svg" | "png",             // action: diagram
  content: "<inline markdown>",      // short sources
  // or:  source: { ref: "sha256:…" },  // the source as a blob ref
  assets: [{ name: "img/logo.png", ref: "sha256:…" }],  // images, by markdown URL
  kind: "mermaid" | "excalidraw",    // diagrams; inferred when omitted
})
```

Exactly one of `content` / `source`.

- **`render`** — markdown with YAML frontmatter → PDF, Word document, or
  slide deck. `mermaid` and `excalidraw` fenced blocks render natively
  (mermaid math labels degrade — this build has no math chain). `toc: true`
  frontmatter adds a Contents outline (PDF) or a static, linked table of
  contents (DOCX).
- **`diagram`** — a mermaid source or raw excalidraw JSON → SVG or PNG.

## Filesystem sessions

When the `docgen-workspace` companion is installed (`docgen_workspace`
appears in the tool table), prefer it for workspace files — one call does
the whole trip, paths in, file out:

```
docgen_workspace({
  action: "render", format: "pdf",
  source: "docs/arch.md",
  assets: [{ name: "img/logo.png", path: "docs/img/logo.png" }],  // name = URL in the md
  out: "docs/build/arch.pdf",          // optional
})
```

Its siblings mint/materialize any blob for hand-offs: `docgen_file_to_blob
{path}` → ref, `docgen_blob_to_file {ref, path}`.

## Flow (blob sessions, no filesystem)

1. Compose the markdown and pass it inline as `content` (or chain a blob
   ref from another tool as `source: {ref}`).
2. Call `docgen_compose`; the result reports the artifact ref and the
   artifact is surfaced to the transcript as a card automatically.
3. For images, pass `assets` — `name` must be the **exact URL the markdown
   uses**. A URL without bytes renders as a visible placeholder and the
   result lists warnings; fix by adding the missing refs.

## Notes

- Prefer `content` for short sources; large sources should ride as blob
  refs so bytes never transit the tool call.
- Renders run under a 30 s / 256 MiB budget per call — split monster
  documents into sections.
- The artifact is a content-addressed digest: pass it on
  (`load_artifact {digest}`, other tools), or materialize it to a file
  when the session has a workspace. With no filesystem it simply stays a
  blob — that is a complete result, not a failure.
