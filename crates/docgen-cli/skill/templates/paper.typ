// Two-column academic paper for docgen — arXiv / journal-article style
// (Universe inspiration: starter-journal-article, splendid-mdpi, arkheion).
// Self-contained: bundled fonts only, no packages.
//
// Optional frontmatter beyond the standard keys: `abstract: "…"` renders
// as an indented abstract block; `keywords: [a, b]` under the abstract.
//
// Build:  docgen render doc.md --template templates/paper.typ -o out.pdf

#import "docgen-meta.typ": meta

#set page(paper: "a4", margin: (x: 1.9cm, y: 2.2cm), numbering: "1")
#set text(font: "Libertinus Serif", size: 10pt, lang: "en")
#set par(justify: true, leading: 0.64em)
#set heading(numbering: none)

// Compact run-in section headings, small-caps-style tracking.
#show heading.where(level: 1): it => {
  set text(size: 1.05em, weight: "bold", tracking: 0.04em)
  block(above: 1.1em, below: 0.5em, it)
}
#show heading.where(level: 2): it => {
  set text(size: 1em, weight: "semibold", style: "italic")
  block(above: 0.9em, below: 0.4em, it)
}
#show heading.where(level: 3): it => {
  set text(size: 1em, weight: "semibold")
  block(above: 0.8em, below: 0.35em, it)
}

#show table: set text(size: 0.9em)
#set table(stroke: 0.5pt + luma(120))
#show raw.where(block: true): block.with(fill: luma(248), inset: 6pt, width: 100%)

// Full-width title block, then the body in two balanced columns.
#if meta.title != none [
  #set align(center)
  #text(size: 1.55em, weight: "bold")[#meta.title]
  #v(0.7em)
  #if meta.authors.len() > 0 [
    #text(size: 0.95em)[#meta.authors.join(" \u{b7} ")]
    #v(0.4em)
  ]
  #let foot = (
    if meta.date != none [#meta.date],
    if meta.version != none [v#meta.version],
  ).filter(p => p != none)
  #if foot.len() > 0 [#text(size: 0.85em, fill: luma(110))[#foot.join(" \u{b7} ")]]
  #v(0.6em)
]

#if meta.at("abstract", default: none) != none [
  #set align(center)
  #text(size: 0.9em, weight: "semibold")[Abstract]
  #set align(left)
  #pad(x: 1.4cm, text(size: 0.9em)[
    #set par(justify: true)
    #meta.abstract
  ])
  #if meta.at("keywords", default: none) != none [
    #pad(x: 1.4cm, text(size: 0.85em, fill: luma(90))[
      *Keywords:* #if type(meta.keywords) == array [#meta.keywords.join(", ")] else [#meta.keywords]
    ])
  ]
  #v(0.8em)
]

#columns(2)[
  #include "docgen-body.typ"
]
