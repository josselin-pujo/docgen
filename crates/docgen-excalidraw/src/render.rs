//! Scene → SVG rendering: roughr for the hand-drawn geometry, embedded
//! fonts shaped into SVG paths for text, embedded data-URLs for images.
//!
//! Everything is deterministic — roughr randomness is seeded from each
//! element's `seed` field, so re-rendering a scene yields identical SVG.

use std::fmt::Write as _;

use roughr::core::{FillStyle, Options, OptionsBuilder};
use roughr::generator::Generator;

use crate::Result;
use crate::fonts::Fonts;
use crate::model::{Binding, Element, Scene};

const PAD: f64 = 8.0;

/// Renders a scene to a standalone SVG string.
pub fn render_svg(scene: &Scene) -> Result<String> {
    let fonts = Fonts::new();
    let elements: Vec<&Element> = scene
        .elements
        .iter()
        .filter(|e| !e.is_deleted && e.kind != "selection" && e.kind != "frame")
        .collect();
    let (min_x, min_y, max_x, max_y) = bounds(&elements);
    let w = (max_x - min_x).max(1.0) + 2.0 * PAD;
    let h = (max_y - min_y).max(1.0) + 2.0 * PAD;

    let mut svg = String::new();
    let bg = &scene.app_state.view_background_color;
    let _ = writeln!(
        svg,
        r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="{w:.0}" height="{h:.0}" viewBox="{:.2} {:.2} {:.2} {:.2}">"#,
        min_x - PAD,
        min_y - PAD,
        w,
        h
    );
    if !bg.is_empty() && bg != "transparent" {
        let _ = writeln!(
            svg,
            r#"<rect x="{:.2}" y="{:.2}" width="{w:.2}" height="{h:.2}" fill="{}"/>"#,
            min_x - PAD,
            min_y - PAD,
            esc_color(bg)
        );
    }

    let generator = Generator::default();
    for el in &elements {
        render_element(&mut svg, el, &generator, &fonts, scene);
    }
    svg.push_str("</svg>\n");
    Ok(svg)
}

fn bounds(elements: &[&Element]) -> (f64, f64, f64, f64) {
    let mut min_x = f64::MAX;
    let mut min_y = f64::MAX;
    let mut max_x = f64::MIN;
    let mut max_y = f64::MIN;
    let mut bump = |x: f64, y: f64| {
        min_x = min_x.min(x);
        min_y = min_y.min(y);
        max_x = max_x.max(x);
        max_y = max_y.max(y);
    };
    for el in elements {
        if el.is_line_like() {
            for (x, y) in el.abs_points() {
                bump(x, y);
            }
        } else {
            bump(el.x, el.y);
            bump(el.x + el.width, el.y + el.height);
        }
    }
    if min_x > max_x {
        return (0.0, 0.0, 100.0, 100.0);
    }
    (min_x, min_y, max_x, max_y)
}

fn render_element(
    svg: &mut String,
    el: &Element,
    generator: &Generator,
    fonts: &Fonts,
    scene: &Scene,
) {
    let opacity = (el.opacity / 100.0).clamp(0.0, 1.0);
    let rotated = el.angle.abs() > 1e-9;
    let (cx, cy) = el.center();
    if opacity < 1.0 || rotated {
        let mut open = String::new();
        if opacity < 1.0 {
            let _ = write!(open, r#" opacity="{:.3}""#, opacity);
        }
        if rotated {
            let _ = write!(
                open,
                r#" transform="rotate({:.2} {:.2} {:.2})""#,
                el.angle.to_degrees(),
                cx,
                cy
            );
        }
        let _ = writeln!(svg, "<g{open}>");
    }

    match el.kind.as_str() {
        "rectangle" => {
            let opts = rough_options(el);
            let d = if el.roundness.is_some() {
                rounded_rect_d(el.x, el.y, el.width, el.height)
            } else {
                String::new()
            };
            let drawable = if el.roundness.is_some() {
                generator.path(d, &Some(opts.clone()))
            } else {
                generator.rectangle(
                    el.x as f32,
                    el.y as f32,
                    el.width as f32,
                    el.height as f32,
                    &Some(opts.clone()),
                )
            };
            emit_drawable(svg, &drawable, el);
        }
        "diamond" => {
            let opts = rough_options(el);
            let pts = diamond_points(el);
            let drawable = generator.polygon(&pts, &Some(opts.clone()));
            emit_drawable(svg, &drawable, el);
        }
        "ellipse" => {
            let opts = rough_options(el);
            let drawable = generator.ellipse(
                (el.x + el.width / 2.0) as f32,
                (el.y + el.height / 2.0) as f32,
                el.width as f32,
                el.height as f32,
                &Some(opts.clone()),
            );
            emit_drawable(svg, &drawable, el);
        }
        "line" | "arrow" => {
            let opts = rough_options(el);
            let abs = anchored_points(el, scene);
            let pts = to_points(&abs);
            if pts.len() >= 2 {
                let drawable = generator.linear_path(&pts, false, &Some(opts.clone()));
                emit_drawable(svg, &drawable, el);
            }
            if el.kind == "arrow" {
                emit_arrowheads(svg, el, &abs, generator);
            }
        }
        "draw" => {
            // Freedraw strokes render as the exact polyline the user drew
            // (excalidraw does not sketch-ify them).
            let pts = el.abs_points();
            if pts.len() >= 2 {
                let mut d = String::new();
                let _ = write!(d, "M{:.2} {:.2}", pts[0].0, pts[0].1);
                for (x, y) in &pts[1..] {
                    let _ = write!(d, " L{x:.2} {y:.2}");
                }
                let _ = writeln!(
                    svg,
                    r#"<path d="{d}" fill="none" stroke="{}" stroke-width="{:.2}" stroke-linecap="round" stroke-linejoin="round" stroke-dasharray="{}"/>"#,
                    esc_color(&el.stroke_color),
                    el.stroke_width,
                    dash(&el.stroke_style, el.stroke_width)
                );
            }
        }
        "text" => {
            let text = el.original_text.as_deref().unwrap_or(&el.text);
            // Bound text (inside a shape) wraps to the container and is
            // vertically aligned within it; standalone text renders at its
            // own box, unwrapped (the app stores it pre-wrapped).
            let container = el
                .container_id
                .as_ref()
                .and_then(|cid| scene.elements.iter().find(|e| &e.id == cid));
            let (x, y, width, wrap) = match container {
                Some(c) => {
                    // Wrap to the shape's inscribed text area: full width
                    // for rectangles/ellipses, narrower for diamonds whose
                    // edges converge where text sits.
                    let ratio = if c.kind == "diamond" { 0.62 } else { 0.92 };
                    let inner = (c.width * ratio).max(12.0);
                    let h = fonts.text_height(
                        text,
                        el.font_family,
                        el.font_size,
                        el.line_height,
                        Some(inner),
                    );
                    let y = match el.vertical_align.as_str() {
                        "top" => c.y + 8.0,
                        "bottom" => c.y + c.height - h - 8.0,
                        _ => c.y + (c.height - h) / 2.0,
                    };
                    (c.x + (c.width - inner) / 2.0, y, inner, true)
                }
                None => (el.x, el.y, el.width, false),
            };
            let (d, _) = fonts.text_to_path(
                text,
                el.font_family,
                el.font_size,
                el.line_height,
                x,
                y,
                &el.text_align,
                width,
                wrap,
            );
            if !d.is_empty() {
                let _ = writeln!(
                    svg,
                    r#"<path d="{}" fill="{}" stroke="none"/>"#,
                    d.trim_end(),
                    esc_color(&el.stroke_color)
                );
            }
        }
        "image" => {
            if let Some(file_id) = &el.file_id
                && let Some(file) = scene.files.get(file_id)
            {
                let _ = writeln!(
                    svg,
                    r#"<image x="{:.2}" y="{:.2}" width="{:.2}" height="{:.2}" href="{}" xlink:href="{}" preserveAspectRatio="none"/>"#,
                    el.x,
                    el.y,
                    el.width,
                    el.height,
                    esc_color(&file.data_url),
                    esc_color(&file.data_url)
                );
            }
        }
        _ => {}
    }

    if opacity < 1.0 || rotated {
        svg.push_str("</g>\n");
    }
}

fn diamond_points(el: &Element) -> Vec<roughr::Point2D<f32>> {
    let (x, y, w, h) = (el.x as f32, el.y as f32, el.width as f32, el.height as f32);
    vec![
        roughr::Point2D::new(x + w / 2.0, y),
        roughr::Point2D::new(x + w, y + h / 2.0),
        roughr::Point2D::new(x + w / 2.0, y + h),
        roughr::Point2D::new(x, y + h / 2.0),
    ]
}

fn to_points(abs: &[(f64, f64)]) -> Vec<roughr::Point2D<f32>> {
    abs.iter()
        .map(|(x, y)| roughr::Point2D::new(*x as f32, *y as f32))
        .collect()
}

/// Absolute arrow/line points with both endpoints re-anchored to their
/// bindings: scenes edited outside the app (elements moved without
/// updating arrow points) still render connected arrows.
fn anchored_points(el: &Element, scene: &Scene) -> Vec<(f64, f64)> {
    let mut pts = el.abs_points();
    if pts.is_empty() {
        // Exporters sometimes omit points when both ends are bound;
        // synthesize them from the bound elements' stored geometry.
        if let (Some(sb), Some(eb)) = (&el.start_binding, &el.end_binding) {
            let s = scene.elements.iter().find(|e| e.id == sb.element_id);
            let e = scene.elements.iter().find(|e| e.id == eb.element_id);
            if let (Some(s), Some(e)) = (s, e) {
                pts = vec![s.center(), e.center()];
            }
        }
        return pts;
    }
    if pts.len() >= 2 {
        if let Some(b) = &el.end_binding
            && let Some(target) = scene.elements.iter().find(|e| e.id == b.element_id)
        {
            let prev = pts[pts.len() - 2];
            let last = *pts.last().unwrap();
            let anchored = bind_point(b, target, last, prev);
            *pts.last_mut().unwrap() = anchored;
        }
        if let Some(b) = &el.start_binding
            && let Some(target) = scene.elements.iter().find(|e| e.id == b.element_id)
        {
            let next = pts[1];
            let first = pts[0];
            pts[0] = bind_point(b, target, first, next);
        }
    }
    pts
}

/// Where a bound arrow endpoint attaches on `target`.
fn bind_point(
    binding: &Binding,
    target: &Element,
    stored: (f64, f64),
    _other: (f64, f64),
) -> (f64, f64) {
    if let Some((fx, fy)) = binding.fixed_point {
        return (target.x + fx * target.width, target.y + fy * target.height);
    }
    let gap = binding.gap.unwrap_or(1.0);
    let mut center = target.center();
    // `focus` displaces the ray origin perpendicular to the center->stored
    // direction (approximation of excalidraw's edge-offset math; exact for
    // focus = 0, which is the overwhelmingly common case).
    if let Some(focus) = binding.focus
        && focus.abs() > 1e-9
    {
        let (dx, dy) = (stored.0 - center.0, stored.1 - center.1);
        let dist = dx.hypot(dy);
        if dist > 1e-9 {
            let (px, py) = (-dy / dist, dx / dist);
            let s = -focus.signum();
            center = (
                center.0 + px * s * focus.abs() * dist,
                center.1 + py * s * focus.abs() * dist,
            );
        }
    }
    perimeter_point(target, center, stored, gap)
}

/// Point where the ray `center -> toward` crosses the target's perimeter,
/// kept `gap` px outside it. Rectangles (and the generic fallback) use the
/// expanded AABB, ellipses the radial solution, diamonds the |x|/a + |y|/b
/// polygon.
fn perimeter_point(
    target: &Element,
    center: (f64, f64),
    toward: (f64, f64),
    gap: f64,
) -> (f64, f64) {
    let (cx, cy) = center;
    let (dx, dy) = (toward.0 - cx, toward.1 - cy);
    let len = dx.hypot(dy);
    if len < 1e-9 {
        return toward;
    }
    let (ux, uy) = (dx / len, dy / len);
    let a = target.width / 2.0 + gap;
    let b = target.height / 2.0 + gap;
    let t = match target.kind.as_str() {
        "ellipse" => {
            let denom = (ux / a) * (ux / a) + (uy / b) * (uy / b);
            if denom < 1e-12 {
                len
            } else {
                denom.sqrt().recip()
            }
        }
        "diamond" => {
            let denom = ux.abs() / a + uy.abs() / b;
            if denom < 1e-12 { len } else { denom.recip() }
        }
        _ => {
            let tx = if ux.abs() > 1e-12 {
                a / ux.abs()
            } else {
                f64::MAX
            };
            let ty = if uy.abs() > 1e-12 {
                b / uy.abs()
            } else {
                f64::MAX
            };
            tx.min(ty)
        }
    };
    (cx + ux * t, cy + uy * t)
}

fn rounded_rect_d(x: f64, y: f64, w: f64, h: f64) -> String {
    let r = (32.0f64).min(w / 2.0).min(h / 2.0);
    let (x2, y2) = (x + w, y + h);
    format!(
        "M{x:.2} {y_plus:.2} A{r:.2} {r:.2} 0 0 1 {x_plus:.2} {y:.2} \
         L{x2_minus:.2} {y:.2} A{r:.2} {r:.2} 0 0 1 {x2:.2} {y_plus:.2} \
         L{x2:.2} {y2_minus:.2} A{r:.2} {r:.2} 0 0 1 {x2_minus:.2} {y2:.2} \
         L{x_plus:.2} {y2:.2} A{r:.2} {r:.2} 0 0 1 {x:.2} {y2_minus:.2} Z",
        x_plus = x + r,
        y_plus = y + r,
        x2_minus = x2 - r,
        y2_minus = y2 - r,
    )
}

/// Two short sketchy strokes at each arrowhead-bearing end.
fn emit_arrowheads(svg: &mut String, el: &Element, pts: &[(f64, f64)], generator: &Generator) {
    if pts.len() < 2 {
        return;
    }
    let size = (14.0 + 3.0 * el.stroke_width).clamp(8.0, 40.0);
    for (head, from_end) in [
        (el.end_arrowhead.as_deref(), true),
        (el.start_arrowhead.as_deref(), false),
    ] {
        let Some(style) = head else { continue };
        if style == "none" {
            continue;
        }
        let (tip, prev) = if from_end {
            (*pts.last().unwrap(), pts[pts.len() - 2])
        } else {
            (pts[0], pts[1])
        };
        let (dx, dy) = (tip.0 - prev.0, tip.1 - prev.1);
        let len = (dx.hypot(dy)).max(1e-6);
        let (ux, uy) = (dx / len, dy / len);
        // Perpendicular + backspread unit vectors.
        let (px, py) = (-uy, ux);
        match style {
            "dot" | "circle" => {
                let r = (el.stroke_width * 3.5).clamp(2.0, 12.0);
                let _ = writeln!(
                    svg,
                    r#"<circle cx="{:.2}" cy="{:.2}" r="{r:.2}" fill="{}" stroke="none"/>"#,
                    tip.0,
                    tip.1,
                    esc_color(&el.stroke_color)
                );
            }
            _ => {
                // Classic arrow / triangle: two strokes flanking the tip.
                let opts = rough_options(el);
                for sign in [-1.0f64, 1.0] {
                    let bx = tip.0 - ux * size + px * sign * size * 0.55;
                    let by = tip.1 - uy * size + py * sign * size * 0.55;
                    let drawable = generator.line(
                        tip.0 as f32,
                        tip.1 as f32,
                        bx as f32,
                        by as f32,
                        &Some(opts.clone()),
                    );
                    emit_drawable(svg, &drawable, el);
                }
            }
        }
    }
}

fn rough_options(el: &Element) -> Options {
    let mut builder = OptionsBuilder::default();
    builder
        .seed(el.seed as u64)
        .roughness(el.roughness as f32)
        .stroke_width(el.stroke_width as f32)
        .stroke(parse_color(&el.stroke_color));
    if !el.background_color.is_empty() && el.background_color != "transparent" {
        builder
            .fill(parse_color(&el.background_color))
            .fill_style(fill_style(&el.fill_style));
    }
    builder.build().unwrap_or_default()
}

fn fill_style(s: &str) -> FillStyle {
    match s {
        "cross-hatch" => FillStyle::CrossHatch,
        "solid" => FillStyle::Solid,
        "zigzag" => FillStyle::ZigZag,
        "dots" => FillStyle::Dots,
        "dashed" => FillStyle::Dashed,
        "zigzag-line" => FillStyle::ZigZagLine,
        _ => FillStyle::Hachure,
    }
}

/// Serializes a roughr drawable as SVG `<path>` elements: fill opsets
/// first, then the sketch (stroke) opset with dash handling.
fn emit_drawable(svg: &mut String, drawable: &roughr::core::Drawable<f32>, el: &Element) {
    for set in &drawable.sets {
        let d = ops_to_d(&set.ops);
        if d.is_empty() {
            continue;
        }
        match set.op_set_type {
            roughr::core::OpSetType::FillPath | roughr::core::OpSetType::FillSketch => {
                let _ = writeln!(
                    svg,
                    r#"<path d="{d}" fill="{}" stroke="none"/>"#,
                    esc_color(&el.background_color)
                );
            }
            _ => {
                let _ = writeln!(
                    svg,
                    r#"<path d="{d}" fill="none" stroke="{}" stroke-width="{:.2}" stroke-linecap="round" stroke-linejoin="round"{}>"#,
                    esc_color(&el.stroke_color),
                    el.stroke_width,
                    dash_attr(&el.stroke_style, el.stroke_width)
                );
                svg.push_str("</path>\n");
            }
        }
    }
}

fn ops_to_d(ops: &[roughr::core::Op<f32>]) -> String {
    use roughr::core::OpType;
    let mut d = String::new();
    for op in ops {
        match op.op {
            OpType::Move => {
                let _ = write!(d, "M{:.2} {:.2} ", op.data[0], op.data[1]);
            }
            OpType::LineTo => {
                let _ = write!(d, "L{:.2} {:.2} ", op.data[0], op.data[1]);
            }
            OpType::BCurveTo => {
                let _ = write!(
                    d,
                    "C{:.2} {:.2} {:.2} {:.2} {:.2} {:.2} ",
                    op.data[0], op.data[1], op.data[2], op.data[3], op.data[4], op.data[5]
                );
            }
        }
    }
    d.trim_end().to_string()
}

fn dash(style: &str, stroke_width: f64) -> String {
    match style {
        "dashed" => format!("{:.2} {:.2}", 8.0 * stroke_width, 5.0 * stroke_width),
        "dotted" => format!(
            "{:.2} {:.2}",
            1.5 * stroke_width.max(1.0),
            5.0 * stroke_width
        ),
        _ => String::new(),
    }
}

fn dash_attr(style: &str, stroke_width: f64) -> String {
    let d = dash(style, stroke_width);
    if d.is_empty() {
        String::new()
    } else {
        format!(r#" stroke-dasharray="{d}""#)
    }
}

fn parse_color(hex: &str) -> roughr::Srgba {
    // palette's Srgba alias is component-f32.
    let h = hex.trim_start_matches('#');
    let (r, g, b, a): (u8, u8, u8, u8) = if h.len() == 3 {
        let v = u16::from_str_radix(h, 16).unwrap_or(0);
        (
            (((v >> 8) & 0xF) * 17) as u8,
            (((v >> 4) & 0xF) * 17) as u8,
            ((v & 0xF) * 17) as u8,
            255,
        )
    } else if h.len() == 6 {
        let v = u32::from_str_radix(h, 16).unwrap_or(0);
        (
            ((v >> 16) & 0xFF) as u8,
            ((v >> 8) & 0xFF) as u8,
            (v & 0xFF) as u8,
            255,
        )
    } else if h.len() == 8 {
        let v = u64::from_str_radix(h, 16).unwrap_or(0xFF);
        (
            ((v >> 24) & 0xFF) as u8,
            ((v >> 16) & 0xFF) as u8,
            ((v >> 8) & 0xFF) as u8,
            (v & 0xFF) as u8,
        )
    } else {
        (0, 0, 0, 255)
    };
    roughr::Srgba::new(
        r as f32 / 255.0,
        g as f32 / 255.0,
        b as f32 / 255.0,
        a as f32 / 255.0,
    )
}

/// Escapes a color/data string for an XML attribute.
fn esc_color(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Binding;

    fn bound_arrow(id: &str, pts: Vec<(f64, f64)>, end: Option<Binding>) -> Element {
        let mut e: Element = serde_json::from_value(serde_json::json!({
            "type": "arrow", "id": id, "x": 0.0, "y": 0.0,
            "points": pts, "seed": 7
        }))
        .unwrap();
        e.end_binding = end;
        e
    }

    fn rect(id: &str, x: f64, y: f64, w: f64, h: f64) -> Element {
        serde_json::from_value(serde_json::json!({
            "type": "rectangle", "id": id, "x": x, "y": y,
            "width": w, "height": h
        }))
        .unwrap()
    }

    #[test]
    fn binding_reanchors_to_moved_rectangle() {
        // Arrow was drawn to the right edge of a box at x=10..200; the box
        // was then moved to x=300 WITHOUT updating the arrow points.
        let scene = Scene {
            elements: vec![
                rect("box", 300.0, 20.0, 190.0, 70.0),
                bound_arrow(
                    "a",
                    vec![(220.0, 55.0), (300.0, 55.0)],
                    Some(Binding {
                        element_id: "box".into(),
                        focus: None,
                        gap: Some(0.0),
                        fixed_point: None,
                    }),
                ),
            ],
            ..Default::default()
        };
        let pts = anchored_points(&scene.elements[1], &scene);
        // End lands on the moved box's left edge, exactly.
        assert!((pts[1].0 - 300.0).abs() < 1e-6, "got {}", pts[1].0);
        assert!((pts[1].1 - 55.0).abs() < 1e-6);
        // The stored (stale) start point is untouched.
        assert_eq!(pts[0], (220.0, 55.0));
    }

    #[test]
    fn fixed_point_pins_the_endpoint() {
        let scene = Scene {
            elements: vec![
                rect("box", 100.0, 40.0, 200.0, 80.0),
                bound_arrow(
                    "a",
                    vec![(0.0, 0.0), (90.0, 90.0)],
                    Some(Binding {
                        element_id: "box".into(),
                        focus: None,
                        gap: None,
                        fixed_point: Some((0.25, 0.75)),
                    }),
                ),
            ],
            ..Default::default()
        };
        let pts = anchored_points(&scene.elements[1], &scene);
        assert_eq!(pts[1], (100.0 + 0.25 * 200.0, 40.0 + 0.75 * 80.0));
    }

    #[test]
    fn ellipse_binding_lands_on_boundary() {
        let scene = Scene {
            elements: vec![
                Element {
                    kind: "ellipse".into(),
                    id: "e".into(),
                    x: 400.0,
                    y: 100.0,
                    width: 160.0,
                    height: 100.0,
                    ..Default::default()
                },
                bound_arrow(
                    "a",
                    vec![(200.0, 150.0), (380.0, 150.0)],
                    Some(Binding {
                        element_id: "e".into(),
                        focus: None,
                        gap: Some(0.0),
                        fixed_point: None,
                    }),
                ),
            ],
            ..Default::default()
        };
        let pts = anchored_points(&scene.elements[1], &scene);
        // (x - cx)²/a² + (y - cy)²/b² == 1 on the boundary.
        let (cx, cy) = (480.0, 150.0);
        let v = (pts[1].0 - cx).powi(2) / 80f64.powi(2) + (pts[1].1 - cy).powi(2) / 50f64.powi(2);
        assert!((v - 1.0).abs() < 1e-6, "boundary value {v}");
    }

    #[test]
    fn unbound_arrow_uses_stored_points() {
        let scene = Scene {
            elements: vec![bound_arrow("a", vec![(1.0, 2.0), (30.0, 40.0)], None)],
            ..Default::default()
        };
        assert_eq!(
            anchored_points(&scene.elements[0], &scene),
            vec![(1.0, 2.0), (30.0, 40.0)]
        );
    }
}
