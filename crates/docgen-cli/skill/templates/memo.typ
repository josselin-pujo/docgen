// Memo / letterhead for docgen — one-pager orientation driven entirely by
// non-standard frontmatter keys, demonstrating the unknown-key passthrough.
// (Universe inspiration: the letter/memo genre.) Self-contained: bundled
// fonts only, no packages.
//
// Frontmatter it expects (all optional, all guarded):
//   to, from, cc, subject, date, ref   — scalars
//   authors                             — signature names
//
// Build:  docgen render doc.md --template templates/memo.typ -o out.pdf

#import "docgen-meta.typ": meta

#set page(paper: "a4", margin: (x: 2.4cm, y: 2.6cm))
#set text(font: "Libertinus Serif", size: 10.5pt, lang: "en")
#set par(justify: true, leading: 0.68em)
#set heading(numbering: none)

#show heading.where(level: 1): it => {
  set text(size: 1.05em, weight: "bold", tracking: 0.03em)
  block(above: 1.2em, below: 0.5em, it)
}
#show table: set text(size: 0.95em)

#let field-row(label, value) = (
  text(size: 0.9em, weight: "semibold", fill: luma(90))[#label],
  text(size: 0.95em)[#value],
)

// -- Letterhead ------------------------------------------------------------
#if meta.title != none [
  #text(size: 1.4em, weight: "bold", fill: rgb("#333"))[#meta.title]
  #v(0.1em)
]
#if meta.subtitle != none [#text(size: 0.95em, fill: luma(110))[#meta.subtitle]]
#v(0.3em)
#line(length: 100%, stroke: 1.2pt + rgb("#333"))
#v(0.9em)

// -- Header fields (unknown-key passthrough in action) ----------------------
#let rows = ()
#if meta.at("to", default: none) != none { rows = rows + field-row([To], meta.to) }
#if meta.at("from", default: none) != none { rows = rows + field-row([From], meta.from) }
#if meta.at("cc", default: none) != none { rows = rows + field-row([cc], meta.cc) }
#if meta.date != none { rows = rows + field-row([Date], meta.date) }
#if meta.at("subject", default: none) != none { rows = rows + field-row([Subject], [*#meta.subject*]) }
#if meta.doc_id != none { rows = rows + field-row([Ref], raw(meta.doc_id)) }

#if rows.len() > 0 [
  #table(
    columns: (3.2cm, 1fr),
    stroke: none,
    inset: (y: 4pt, x: 0pt),
    ..rows.flatten(),
  )
  #v(0.4em)
  #line(length: 100%, stroke: 0.5pt + luma(200))
  #v(0.8em)
]

// -- Body --------------------------------------------------------------------
#include "docgen-body.typ"

// -- Signature block ----------------------------------------------------------
#v(2em)
#if meta.authors.len() > 0 [
  #line(length: 30%, stroke: 0.7pt + luma(130))
  #v(0.3em)
  #text(size: 0.95em)[#meta.authors.join(" \u{b7} ")]
]
