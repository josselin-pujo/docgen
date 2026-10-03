// Example custom typst template for docgen.
//
// docgen compiles your markdown to two virtual modules:
//   - "docgen-body.typ"  the document's blocks (headings, paragraphs, ...)
//   - "docgen-meta.typ"  `meta`, the frontmatter as a dictionary
//                        (title, subtitle, authors, date, version, doc_id,
//                        plus every unknown key you wrote)
//
// This file becomes main.typ; build the whole document shell here and place
// the body with #include.

#import "docgen-meta.typ": meta

#set page(paper: "a4", margin: (x: 2.5cm, y: 3cm), numbering: "1 / 1")
#set heading(numbering: "1.1")
#set text(size: 10.5pt)

// Header strip with the doc title on every page but the first.
#context {
  if counter(page).get().first() > 1 [
    #set align(center)
    #text(size: 8.5pt, fill: luma(120))[_#meta.title _]
  ]
}

// Title block.
#block(fill: rgb("#1e3a5f"), inset: 14pt, radius: 6pt, width: 100%)[
  #set align(center)
  #text(fill: white, size: 1.5em, weight: "bold")[#meta.title]
  #v(0.4em)
  #text(fill: rgb("#cdd9ea"))[#meta.subtitle]
]
#v(0.8em)
#align(center)[#text(size: 0.85em, fill: luma(110))[
  #meta.authors.join(" · ") · #meta.date · #meta.version
]]
#v(1.5em)

// The document itself.
#include "docgen-body.typ"
