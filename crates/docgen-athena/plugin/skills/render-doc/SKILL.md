---
name: render-doc
description: Render markdown to PDF/DOCX/PPTX and mermaid/excalidraw diagrams to SVG/PNG, fully in-process — works without shell or unix tools.
---

# render-doc

`docgen_compose` renders documents and diagrams inside this session — a
wasm component runs the real docgen pipeline (the same markdown, YAML
frontmatter, and diagram support as the `docgen` CLI, without its diff
features). No executables, no shell: it works in sessions that have no
unix-tools.

## Call

```
docgen_compose({
  action: "render" | "diagram",
  format: "pdf" | "docx" | "pptx",   // action: render
  // or   "svg" | "png",             // action: diagram
  source: "doc.md",                  // workspace file — preferred
  // or:  content: "<inline text>",  // short sources only
  kind: "mermaid" | "excalidraw",    // diagrams; inferred when omitted
  out: "doc.pdf",                    // optional; defaults beside the source
})
```

Exactly one of `source` / `content`.

- **`render`** — markdown with YAML frontmatter → PDF, Word document, or
  slide deck. `mermaid` and `excalidraw` fenced blocks render natively
  (mermaid math labels degrade — this build has no math chain).
- **`diagram`** — a mermaid source or raw excalidraw JSON → SVG or PNG.

## Flow

1. Get the source into the workspace (write it to a file) — or pass it
   inline as `content`.
2. Call `docgen_compose`.
3. The result reports the written file (e.g. `wrote doc.pdf (pdf)`); the
   artifact is also surfaced to the transcript as a card automatically —
   no separate `load_artifact` call is needed.
4. When the user will keep working with the artifact, mention the
   workspace path from the result.

## Notes

- Prefer `source` for anything sizable; inline `content` rides the tool
  call and is best kept short.
- Renders run under a 30 s / 256 MiB budget per call — split monster
  documents into sections.
- The output must be written to the workspace (the plugin needs its
  `fs.write` grant at install time); if writing is unavailable, the
  result says so and carries the artifact digest instead.
