# Font licenses — `../assets`

The TTFs in this directory are standalone-TTF conversions of the latin
woff2 subsets shipped in the `@excalidraw/excalidraw` npm package
(`dist/prod/fonts/*`); see `../FONTS.md` for the conversion details and
which `fontFamily` each file serves. This directory holds the license
texts that must accompany any redistribution of those fonts.

| File | License | Copyright / Reserved Font Names (per upstream) |
|---|---|---|
| `Excalifont-Regular.ttf` | [MIT](MIT.txt) | Excalifont — © Excalidraw, license per <https://excalifont.com> (verify & vendor the upstream text verbatim if it differs) |
| `Virgil.ttf` | [OFL-1.1](OFL-1.1.txt) | © 2021–present Ellinor Rapp, Reserved Font Name **Virgil** (per `excalidraw/virgil` LICENSE.md) |
| `Nunito.ttf` | [OFL-1.1](OFL-1.1.txt) | © 2011 The Nunito Project Authors |
| `Cascadia.ttf` | [OFL-1.1](OFL-1.1.txt) | © 2019–present Microsoft Corporation, Reserved Font Name **Cascadia Code** |
| `LiberationSans.ttf` | [OFL-1.1](OFL-1.1.txt) | © 2012 Red Hat, Inc. (Liberation Fonts project) |

Notes:

- The canonical OFL 1.1 text is [OFL-1.1.txt](OFL-1.1.txt) (identical to
  the copy embedded in `third-party/NOTICE.typst-assets` in the repo root).
- These files are *subset conversions* of the upstream fonts, so the OFL
  "Modified Version" rules apply: do not use the Reserved Font Names for
  further derivatives, and keep this directory with the fonts.
- `Virgil.ttf` was previously listed as Apache-2.0 in `../FONTS.md`; the
  upstream `excalidraw/virgil` repository licenses it under OFL-1.1
  (verified via the upstream LICENSE.md, 2026-10-17).
