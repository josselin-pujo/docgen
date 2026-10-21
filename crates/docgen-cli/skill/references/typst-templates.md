# Typst template authoring primer

How to write a custom PDF template for docgen: what a template is, what
it can see, what the body looks like, and a working tour of the typst
constructs templates need. docgen embeds **typst 0.15**; the official
tutorial and language reference at <https://typst.app/docs/> apply
(different entry point — typst.app compiles `.typ` files directly,
docgen feeds it a generated document — but the language is identical).

Read this before writing or editing a `--template` file.

## The model

`docgen render doc.md --template shell.typ` compiles your file as the
typst main file. Three virtual modules exist inside the compile:

| Virtual file | What it is |
|---|---|
| `main.typ` | your template (plus up to two generated `#set document(...)` metadata lines prepended from frontmatter) |
| `docgen-meta.typ` | exposes `meta` — the frontmatter as a dictionary |
| `docgen-body.typ` | the rendered document blocks |

A template is ordinary typst that imports the metadata and places the
body:

```typst
#import "docgen-meta.typ": meta

#set page(paper: "a4", margin: 2.5cm, numbering: "1")
#set heading(numbering: "1.1")
#set text(font: "Libertinus Serif", size: 11pt, lang: "en")

// ... cover, headers, footers, show rules ...

#include "docgen-body.typ"
```

## The `meta` dictionary

Always complete: typed keys are `none` when unset, so templates can
guard with `!= none`. Unknown YAML keys pass through with full structure
(nested dictionaries, arrays, numbers, booleans — YAML → typst literal).

| Key | Type |
|---|---|
| `title`, `subtitle`, `date`, `version`, `doc_id` | string or `none` |
| `authors` | array of strings (possibly empty) |
| `toc` | boolean (`meta.at("toc", default: false)`) |
| `media` | dictionary (`placement`, `index`) |
| *anything else you wrote* | as written |

Use the extras freely — a template can require its own frontmatter keys
(`department`, `reviewers`, `to`/`from` for a memo) and fail with a
clear typst error when absent.

The **body is opaque content**: it contains no cover, no outline — the
template owns those. If you want `toc: true` to keep working, render the
outline yourself:

```typst
#if meta.at("toc", default: false) [
  #outline(title: [Contents], depth: 3)
  #v(1.5em)
]
```

## What the body emits

Knowing the exact markup lets show/set rules restyle it. The body is
plain typst markup:

- headings: `= Title <section-id>` — labels carry the markdown ids, so
  internal links (`#link(<…>)[…]`) resolve regardless of styling.
- paragraphs: plain text, fully escaped author content.
- lists: `- item` / `1. item` with indentation for nesting.
- tables: `#table(columns: N, table.header([..], ..), [..], ..)`.
- block quotes: `#quote(block: true)[ … ]`.
- code: raw fenced blocks with the language tag.
- horizontal rules: `#line(length: 100%)`.
- strikethrough: `#strike[ … ]`.
- figures: `#align(center)[#image("assets/<name>")]`, captioned ones
  wrapped in `#block(breakable: false)[ … ]` with a centered
  "Figure N: caption" line; each carries a `<fig-…>` label.
- appendix / list of figures: level-1 unnumbered headings
  (`#heading(numbering: none, level: 1)[Figures]`) and
  `#context[#link(<fig-…>)[Figure N] … p. #query(<fig-…>)…]` index
  lines.

So the high-leverage styling hooks are: `#set page`, `#set text`,
`#set heading` / `#show heading`, `#show table`, `#show quote`,
`#set par`, and `#show raw`/`#show raw.where(block: true)`. Figure
captions carry their own small centered styling and the media appendix
structure is fixed — restyle those by targeting the surrounding
elements, not by rewriting the body.

## Hard constraints

- **Hermetic world**: no network, no package store — `#import
  "@preview/…"` cannot work, and neither can `#read`/`#image` of files
  outside the virtual set (`docgen-meta.typ`, `docgen-body.typ`,
  `assets/*`). Templates are self-contained typst.
- **Fonts**: only the bundled families exist — **Libertinus Serif**
  (regular/semibold/bold/italics), **NewCM10** (Computer-Modern-style
  serif), **NewCMMath**, **DejaVu Sans Mono**. There is **no sans
  family**: build contrast with size, weight, color, letter-spacing —
  unknown families fall back with a typst warning.
- **No clock**: `datetime.today()` is `none`; every date must come from
  `meta`. (This is what makes output byte-deterministic.)
- **Page control is a built-in-template feature**: with a custom
  template, `new-page`, `page=` and fit-to-page scaling in the source
  are inert (the template owns geometry; images get a fixed 92% width).
  `place=appendix` still works — figures render in the back matter
  without the extra page breaks — and the figures index still emits.

## Language tour for template authors

Code lives after `#`; markup is the rest. The constructs below cover
essentially every template need.

**Set rules** — properties for everything after them:

```typst
#set page(paper: "a4", margin: (x: 2.2cm, y: 2.6cm),
          numbering: "1 / 1",                 // footer pattern
          header: context text(size: 8.5pt)[
            #if counter(page).get().first() > 1 [#meta.title #h(1fr)]
          ])
#set text(font: "Libertinus Serif", size: 10.5pt, lang: "en")
#set par(justify: true, leading: 0.65em)
#set heading(numbering: "1.1")                // numbered sections
#set enum(numbering: "a.")                    // ordered-list style in the body
```

**Show rules** — transform elements:

```typst
#show heading.where(level: 1): it => {
  set text(size: 1.45em, weight: "bold", fill: rgb("#1e3a5f"))
  block(above: 1.6em, below: 1em, it)
}
#show heading.where(level: 1): set block(above: 1.4em)
#show raw.where(block: true): block.with(
  fill: luma(245), inset: 8pt, radius: 4pt, width: 100%)
#show quote: set text(style: "italic")
#show table: set text(size: 0.92em)
#show table: set table.stroke(luma(120))
```

**Variables, control flow, content**:

```typst
#let accent = rgb("#0f6b4f")
#let band(body) = block(fill: accent, inset: 12pt, radius: 4pt, width: 100%,
                        text(fill: white, body))
#if meta.title != none [ #band[#meta.title] ]
#for a in meta.authors [#if !loop.first [, ]#a]
```

**Context, counters, queries** — for headers/footers that know where
they are:

```typst
#context [
  #set align(right)
  Page #context counter(page).display("1 of 1")
]
// running section name in the header:
#context {
  let secs = query(heading.where(level: 1).before(here()))
  if secs.len() > 0 { secs.last().body }
}
```

**Helper templates** — the typst "template" pattern: a function that
takes content and returns styled content; apply it with
`#show: project` if you want a fully packaged look:

```typst
#let project(body) = {
  set page(paper: "a4", margin: 2.2cm, numbering: "1")
  set text(font: "Libertinus Serif", size: 10.5pt)
  set heading(numbering: "1.1")
  // cover from meta here
  body
}
#show: project
#include "docgen-body.typ"
```

Numbering patterns (`"1.1"`, `"1 / 1"`, `"I"`, `"a)"`) and full
parameter references are in the typst reference — `page`, `text`,
`heading`, `table`, `outline`, `state`, `counter` chapters.

## Diagnostics and the edit loop

Compile errors come back with virtual file and line:
`main.typ:12` is your template (count the generated `#set document`
lines prepended at the top — usually none, at most two), and
`docgen-meta.typ` / `docgen-body.typ` name themselves. Typst warnings
(unknown font, missing label) surface alongside docgen's own warnings
on stderr — read them, they usually name the exact element.

Test loop: keep a rich sample markdown (headings, table, list, quote,
code, a captioned mermaid figure, internal links, `toc: true`) and

```bash
docgen render sample.md --template templates/report.typ -o sample.pdf
```

## Bundled sample templates

Package-free, hermetic, verified with the shipped binary — use directly
or as starting points (`docgen render doc.md --template
templates/<name>.typ`):

| Template | Genre (inspired by typst Universe staples) | Demonstrates |
|---|---|---|
| `templates/report.typ` | classic report / thesis | cover page, numbered headings, running headers, "Page N of M", Contents |
| `templates/paper.typ` | two-column academic paper (arXiv/MDPI style) | columns, author block from meta, unnumbered styling, compact tables |
| `templates/design-doc.typ` | modern engineering design doc | accent color system, title band, metadata footer with `doc_id`/`version`, boxed raw blocks |
| `templates/handbook.typ` | manual / booklet | chapter-per-H1 display, running section header via `query`, outline styling |
| `templates/memo.typ` | memo / letterhead | unknown-meta keys (`to`/`from`/`cc`/`subject`), ruled header, signature block |

Start from `report.typ` — it is the annotated one.
