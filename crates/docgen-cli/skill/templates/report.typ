// Classic report / thesis template for docgen.
//
// Genre inspiration: the university-thesis and official-report staples on
// the typst Universe — cover page, numbered sections, running headers,
// "Page N of M". Self-contained on purpose: docgen's hermetic typst world
// has no package store and only the bundled fonts (Libertinus Serif,
// NewCM10, DejaVu Sans Mono).
//
// Build:  docgen render doc.md --template templates/report.typ -o out.pdf
//
// This is the annotated sample; the other templates demonstrate specific
// techniques without the commentary.

#import "docgen-meta.typ": meta

// -- Page setup -----------------------------------------------------------
// Numbering stays "1" — typst treats lowercase `a`/`i` inside a numbering
// pattern as counting symbols ("Page 1" prints "Pbge 2", the Contents shows
// "Pb"), so the "Page N of M" wording lives in a footer function instead.
// The header shows the document title from page 2 on.
#set page(
  paper: "a4",
  margin: (x: 2.5cm, y: 3cm),
  numbering: "1",
  footer: context {
    if counter(page).get().first() > 1 {
      set text(size: 9pt, fill: luma(110))
      let total = counter(page).final().first()
      align(center)[Page #counter(page).display() of #total]
    }
  },
  header: context {
    if counter(page).get().first() > 1 {
      set text(size: 8.5pt, fill: luma(100))
      [#meta.title #h(1fr)]
      line(length: 100%, stroke: 0.5pt + luma(180))
      v(0.4em)
    }
  },
)

// -- Base typography ------------------------------------------------------
#set text(font: "Libertinus Serif", size: 11pt, lang: "en")
#set par(justify: true, leading: 0.66em)
#set heading(numbering: "1.1") // the body's headings become "1", "1.1", …

// Section headings in the house color, with controlled spacing.
#show heading.where(level: 1): it => {
  set text(size: 1.5em, weight: "bold", fill: rgb("#1e3a5f"))
  block(above: 1.6em, below: 0.9em, it)
}
#show heading.where(level: 2): it => {
  set text(size: 1.2em, weight: "bold", fill: rgb("#1e3a5f"))
  block(above: 1.3em, below: 0.7em, it)
}
#show heading.where(level: 3): it => {
  set text(size: 1.05em, weight: "semibold")
  block(above: 1.1em, below: 0.6em, it)
}

// Code blocks in a quiet panel; tables slightly smaller with hairline grid.
#show raw.where(block: true): block.with(
  fill: luma(246), inset: 8pt, radius: 3pt, width: 100%, breakable: true,
)
#show table: set text(size: 0.92em)
#set table(stroke: luma(150))

// -- Cover page -----------------------------------------------------------
#if meta.title != none or meta.subtitle != none [
  #set align(center)
  #v(6em)
  #if meta.title != none [
    #text(size: 2em, weight: "bold")[#meta.title]
    #v(0.6em)
  ]
  #if meta.subtitle != none [
    #text(size: 1.2em, fill: luma(80))[#emph[#meta.subtitle]]
    #v(3em)
  ]
  #let ident = (
    if meta.authors.len() > 0 [#meta.authors.join(" \u{b7} ")],
    if meta.date != none [#meta.date],
    if meta.version != none [v#meta.version],
  ).filter(p => p != none)
  #if ident.len() > 0 [#text(size: 0.95em, fill: luma(110))[#ident.join(" \u{b7} ")]]
  #v(2em)
  #if meta.doc_id != none [#text(size: 0.85em, fill: luma(140), font: "DejaVu Sans Mono")[#meta.doc_id]]
  #pagebreak()
]

// -- Contents (keeps the `toc: true` frontmatter working) ------------------
#if meta.at("toc", default: false) [
  #outline(title: text(size: 1.5em, weight: "bold")[Contents], depth: 3)
  #v(1.5em)
]

// -- The document ----------------------------------------------------------
#include "docgen-body.typ"
