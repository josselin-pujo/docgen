// Modern engineering design doc for docgen — accent color system, title
// band, metadata footer. (Universe inspiration: the minimal-report and
// ADR/design-doc genre.) Self-contained: bundled fonts only, no packages.
//
// Optional frontmatter extras, rendered in a status table when present:
//   status: Draft | In review | Accepted
//   reviewers: [name, name]
//   team: Platform
//
// Build:  docgen render doc.md --template templates/design-doc.typ -o out.pdf

#import "docgen-meta.typ": meta

#let accent = rgb("#0f6b4f")
#let accent-soft = rgb("#e7f3ee")
#let ink = rgb("#202124")

#set page(
  paper: "a4",
  margin: (x: 2.2cm, y: 2.6cm),
  numbering: "1",
  footer: context {
    if counter(page).get().first() > 1 {
      set text(size: 8pt, fill: luma(120))
      line(length: 100%, stroke: 0.5pt + luma(200))
      v(0.3em)
      [#meta.at("doc_id", default: "document") #h(1fr) #if meta.version != none [v#meta.version ]#counter(page).display("1")]
    }
  },
)
#set text(font: "Libertinus Serif", size: 10.5pt, fill: ink, lang: "en")
#set par(justify: false, leading: 0.65em)
#set heading(numbering: none)

#show heading.where(level: 1): it => {
  set text(size: 1.35em, weight: "bold", fill: accent)
  block(above: 1.5em, below: 0.8em, {
    set text(size: 0.35em, weight: "regular", fill: accent)
    box(stroke: (bottom: 1pt + accent), width: 100%, inset: (bottom: 4pt), it)
  })
}
#show heading.where(level: 2): it => {
  set text(size: 1.1em, weight: "bold", fill: ink)
  block(above: 1.2em, below: 0.55em, it)
}
#show heading.where(level: 3): it => {
  set text(size: 1em, weight: "semibold", fill: accent.darken(10%))
  block(above: 1em, below: 0.45em, it)
}

#show raw.where(block: true): block.with(
  fill: luma(246), stroke: (left: 2.5pt + accent), inset: 9pt,
  radius: (top-right: 3pt, bottom-right: 3pt), width: 100%, breakable: true,
)
#show quote: set block(fill: accent-soft, inset: 10pt, radius: 3pt)
#show table: set text(size: 0.9em)
#set table(stroke: luma(170))

// -- Title band ------------------------------------------------------------
#block(fill: ink, inset: 14pt, radius: 4pt, width: 100%)[
  #set align(left)
  #text(fill: white, size: 0.75em, tracking: 0.18em)[DESIGN DOCUMENT]
  #v(0.3em)
  #if meta.title != none [
    #text(fill: white, size: 1.5em, weight: "bold")[#meta.title]
  ]
  #if meta.subtitle != none [
    #v(0.2em)
    #text(fill: rgb("#c8d4cf"), size: 0.95em)[#meta.subtitle]
  ]
]
#v(0.8em)

// -- Status line / optional fields -----------------------------------------
// Built with explicit pushes: `if cond (a, b)` inside a collection parses
// as a call on the condition (`0 (…)`), so keep the branches braced.
#let fields = ()
#if meta.authors.len() > 0 { fields.push(([Authors], meta.authors.join(", "))) }
#if meta.date != none { fields.push(([Date], meta.date)) }
#if meta.at("status", default: none) != none { fields.push(([Status], meta.status)) }
#if meta.at("team", default: none) != none { fields.push(([Team], meta.team)) }
#if meta.at("reviewers", default: none) != none {
  fields.push(([Reviewers], if type(meta.reviewers) == array {
    meta.reviewers.join(", ")
  } else {
    meta.reviewers
  }))
}
#if meta.version != none { fields.push(([Version], [v#meta.version])) }
#if meta.doc_id != none {
  fields.push(([ID], text(font: "DejaVu Sans Mono", size: 0.9em)[#meta.doc_id]))
}

#if fields.len() > 0 [
  #table(
    columns: (auto, 1fr),
    stroke: none,
    inset: (y: 3pt, x: 0pt),
    ..fields.map(p => (
      text(size: 0.85em, weight: "semibold", fill: accent)[#p.at(0)],
      text(size: 0.9em)[#p.at(1)],
    )).flatten(),
  )
  #v(0.6em)
]

#if meta.at("toc", default: false) [
  #outline(title: text(fill: accent, size: 1.2em, weight: "bold")[Contents], depth: 3, indent: 1em)
  #v(1em)
]

#include "docgen-body.typ"
