# Bundled fonts

Converted to standalone TTF (from woff2 subsets shipped in the
`@excalidraw/excalidraw` npm package, `dist/prod/fonts/*`) with fonttools.
Each file is the largest subset of its family (latin coverage).

| File | Excalidraw `fontFamily` | License |
|---|---|---|
| `Excalifont-Regular.ttf` | 4 (current default) | MIT — https://excalifont.com |
| `Virgil.ttf` | 1 (legacy handwriting) | OFL 1.1 — excalidraw/virgil (© Ellinor Rapp, RFN Virgil) |
| `Nunito.ttf` | 2 (normal sans) | OFL 1.1 |
| `Cascadia.ttf` | 3 (code) | OFL 1.1 |
| `LiberationSans.ttf` | 2 fallback metrics | OFL 1.1 |

License texts and per-font copyright/RFN details live in
[`LICENSES/`](LICENSES/README.md) and must accompany any redistribution
of these fonts.

The renderer maps every `fontFamily` code to one of these; unmapped or
subset-missing glyphs fall back across them in that order.
