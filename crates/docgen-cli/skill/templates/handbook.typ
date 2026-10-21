// Handbook / manual for docgen — chapter-per-H1 display with running
// section names in the header. (Universe inspiration: the book/manual
// genre.) Self-contained: bundled fonts only, no packages.
//
// Build:  docgen render doc.md --template templates/handbook.typ -o out.pdf

#import "docgen-meta.typ": meta

#set page(
  paper: "a4",
  margin: (x: 2.6cm, top: 3.2cm, bottom: 2.8cm),
  numbering: "1",
  header: context {
    if counter(page).get().first() > 1 {
      let secs = query(heading.where(level: 1).before(here()))
      set text(size: 8.5pt, fill: luma(110))
      if meta.title != none [#meta.title #h(1fr)]
      if secs.len() > 0 [— #secs.last().body]
      v(-0.9em)
      line(length: 100%, stroke: 0.5pt + luma(200))
    }
  },
)
#set text(font: "Libertinus Serif", size: 10.5pt, lang: "en")
#set par(justify: true, leading: 0.68em)
// Number only level 1: the counter steps for every heading, but a numbering
// function returning none keeps deeper levels clean. With `numbering: none`
// the counter would never step at all, and the chapter eyebrow could not
// count.
#set heading(numbering: (..n) => if n.pos().len() == 1 { numbering("1", ..n) })

// Chapters: centered display block with "Chapter N" above the title.
// The weak pagebreak must sit outside the block container — typst forbids
// pagebreaks inside containers.
#show heading.where(level: 1): it => {
  set text(fill: rgb("#4a3423"))
  pagebreak(weak: true)
  block(above: 2em, below: 1.2em, width: 100%, breakable: false)[
    #set align(center)
    #text(size: 0.8em, tracking: 0.25em, fill: luma(130))[
      CHAPTER #counter(heading).display("1")
    ]
    #v(0.4em)
    #text(size: 1.7em, weight: "bold")[#it.body]
    #v(0.2em)
    #align(center, line(length: 30%, stroke: 1pt + luma(160)))
  ]
}
#show heading.where(level: 2): it => {
  set text(size: 1.15em, weight: "semibold", fill: rgb("#4a3423"))
  block(above: 1.3em, below: 0.6em, it)
}
#show heading.where(level: 3): it => {
  set text(size: 1em, weight: "semibold", style: "italic")
  block(above: 1em, below: 0.45em, it)
}

#show raw.where(block: true): block.with(
  fill: rgb("#faf6f0"), inset: 9pt, radius: 3pt, width: 100%, breakable: true,
)
#show table: set text(size: 0.92em)
#set table(stroke: 0.5pt + rgb("#c9b8a3"))

#if meta.at("toc", default: false) [
  // `outline(title: …)` renders the title as a level-1 heading — it would
  // hit the chapter show rule and steal a chapter number. Draw our own.
  #text(size: 1.4em, weight: "bold", fill: rgb("#4a3423"))[Contents]
  #v(0.6em)
  #outline(title: none, depth: 2, indent: 1.2em)
  #v(1.2em)
]

#include "docgen-body.typ"
