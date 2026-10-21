# Excalidraw primer

Raw `.excalidraw` JSON is a first-class input: standalone files
(`docgen diagram scene.excalidraw`) and fenced ` ```excalidraw ` blocks
in markdown. Scenes render natively — hand-drawn geometry via roughr,
embedded fonts shaped to SVG paths, no browser, byte-deterministic
(each element's `seed` drives its randomness). The JSON model is
permissive: unknown fields round-trip losslessly, unknown element kinds
are skipped silently — authoring against the real excalidraw app is
always safe.

Read this before generating scene JSON. The renderer honors the fields
listed below and ignores the rest.

## Scene shape

```json
{
  "type": "excalidraw",
  "version": 2,
  "source": "docgen",
  "elements": [ … ],
  "appState": { "viewBackgroundColor": "#ffffff" },
  "files": { "img1": { "dataURL": "data:image/png;base64,…" } }
}
```

`appState.viewBackgroundColor` becomes the canvas background (skip it or
use `"transparent"` for none). `files` holds image payloads keyed by id;
images reference them via `fileId`.

## Elements

Coordinates are y-down; the canvas auto-crops to the element bounds with
8px padding. Shared style fields on every element: `strokeColor`,
`backgroundColor` (+ `fillStyle`), `strokeWidth`, `strokeStyle`
(`solid` | `dashed` | `dotted`), `roughness` (0 = architect, 1 = artist,
2 = cartoonist), `opacity` (0–100), `angle` (radians, rotation about the
center), `seed` (any integer — fixes the hand-drawn wobble), `id`,
`isDeleted`.

| Kind | Honored geometry / notes |
|---|---|
| `rectangle` | x, y, width, height; `roundness` present → rounded corners |
| `diamond` | polygon through the four edge midpoints |
| `ellipse` | width/height as diameters |
| `line`, `arrow` | `points` (relative to x/y); arrows draw heads from `startArrowhead`/`endArrowhead` (`arrow`/`triangle` → classic head, `dot`/`circle` → dot) |
| `draw` | freedraw: rendered as the exact stored polyline |
| `text` | see below |
| `image` | x/y/width/height, `fileId` → `files[id].dataURL` |

`frame` and `selection` elements are ignored.

**Fill styles**: `hachure` (default), `cross-hatch`, `solid`, `zigzag`,
`dots`, `dashed`, `zigzag-line`. Fill only applies when
`backgroundColor` is set and not `"transparent"`.

## Text

Two forms:

- **Standalone** (`containerId` absent): renders at its own box,
  `textAlign` honored, *not* re-wrapped — write it pre-wrapped (the
  excalidraw app stores it that way; when hand-authoring, insert your
  own `\n`).
- **Bound** (a `text` element with `containerId: "<shape id>"` plus the
  shape itself): wraps inside the container (92% of its width; 62% for
  diamonds), `verticalAlign` `top`/`middle`/`bottom` positions it, the
  container is the label's frame. **Prefer bound text for labels** —
  they stay attached when the shape moves.

`fontFamily` codes: `1` = Virgil (hand-drawn), `2` = normal (Liberation
Sans), `3` = code (Cascadia Mono), `4` or missing = Excalifont (the
current excalidraw default). `fontSize` in px, `lineHeight` defaults to
1.25.

## Bindings (arrows that stay attached)

An arrow endpoint with a binding re-anchors at render time to the bound
shape — scenes edited outside the app still render connected:

```json
{ "type": "arrow", "id": "a1", "x": 150, "y": 60,
  "width": 70, "height": 0, "points": [[0,0],[70,0]],
  "startBinding": { "elementId": "box1", "focus": 0, "gap": 4 },
  "endBinding":   { "elementId": "box2", "focus": 0, "gap": 4 },
  "endArrowhead": "arrow" }
```

- `fixedPoint: [fx, fy]` (fractions of the target's width/height) is
  exact — the arrow tip lands there.
- Otherwise the point is pulled to the target's perimeter along the
  center ray with `gap` (px) subtracted; `focus` displaces the ray
  (exact for `focus: 0` — use that unless you need edge offsets).
- Set bindings on **both ends**; if the exporter omitted `points`
  entirely, centers are synthesized.

## Element templates

Copy-paste starting points (minimum viable fields; add style fields as
needed — missing ones get sane defaults).

Box with bound label:

```json
{ "type": "rectangle", "id": "box1", "x": 40, "y": 40, "width": 160, "height": 60,
  "strokeColor": "#1e1e1e", "backgroundColor": "#a5d8ff", "fillStyle": "solid",
  "strokeWidth": 2, "roughness": 1, "opacity": 100, "seed": 11, "roundness": null },
{ "type": "text", "id": "box1-label", "x": 60, "y": 60, "width": 120, "height": 25,
  "text": "Service", "fontSize": 20, "fontFamily": 4, "textAlign": "center",
  "verticalAlign": "middle", "containerId": "box1", "strokeColor": "#1e1e1e",
  "opacity": 100, "seed": 12 }
```

Decision diamond + arrow:

```json
{ "type": "diamond", "id": "dec1", "x": 260, "y": 30, "width": 140, "height": 80,
  "strokeColor": "#1e1e1e", "backgroundColor": "#ffec99", "fillStyle": "solid",
  "strokeWidth": 2, "roughness": 1, "opacity": 100, "seed": 21, "roundness": null },
{ "type": "arrow", "id": "e1", "x": 200, "y": 70, "width": 60, "height": 0,
  "points": [[0, 0], [60, 0]], "strokeColor": "#1e1e1e", "strokeWidth": 2,
  "roughness": 1, "opacity": 100, "seed": 22,
  "startBinding": { "elementId": "box1", "focus": 0, "gap": 4 },
  "endBinding": { "elementId": "dec1", "focus": 0, "gap": 4 },
  "startArrowhead": null, "endArrowhead": "arrow" }
```

Standalone note text:

```json
{ "type": "text", "id": "note1", "x": 40, "y": 130, "width": 180, "height": 50,
  "text": "line one\nline two", "fontSize": 16, "fontFamily": 4,
  "textAlign": "left", "strokeColor": "#e03131", "opacity": 100, "seed": 31 }
```

Image (bytes live in `files`):

```json
{ "type": "image", "id": "img1", "x": 40, "y": 200, "width": 120, "height": 80,
  "fileId": "img1", "strokeColor": "transparent", "backgroundColor": "transparent",
  "opacity": 100, "seed": 41 }
```

## Conventions for generated scenes

These keep renders clean and diffs meaningful:

- **Descriptive string ids** (`"svc-auth"`, not `"el_27"`) — diffing is
  element-id-anchored and changed *fields* are enumerated.
- **Grid discipline**: consistent box sizes, aligned rows/columns,
  ≥ 40px gutters. The renderer does not tidy up for you.
- **Bind every arrow** to its shapes (both ends, `focus: 0`); free
  arrow tips are for annotations only.
- **Labels belong to shapes** (bound text), captions/legend as
  standalone text.
- **Seed discipline**: any distinct integer per element; the same seed
  renders the same wobble forever.
- One diagram = one argument. If a scene needs a legend to explain the
  legend, split it.
- The canvas is the bounding box of the elements: a stray far-away
  element inflates the whole figure when it scales to fit the page.

## Workflow

1. Write the scene JSON (fenced ` ```excalidraw ` block or a
   `.excalidraw` file).
2. Check it visually: `docgen diagram scene.excalidraw --format png -o
   scene.png` and view the PNG.
3. Iterate on geometry/labels; then embed (fence in markdown, or
   reference the file).
4. In documents, give it a caption and id:
   ```` ```excalidraw {#fig:overview caption="System overview"} ````.

Fidelity notes: rich text (per-span formatting), auto-growing
containers and `frame` elements are not rendered; diamond/ellipse
roundness and pressure-width freedraw strokes are approximated. Keep
text simple and geometry explicit and renders match the app closely.
