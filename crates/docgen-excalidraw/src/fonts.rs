//! Embedded fonts and text-to-SVG-path shaping via ab_glyph.
//!
//! Excalidraw's `fontFamily` codes map onto the bundled TTFs (see
//! `assets/FONTS.md`); unknown codes and glyphs missing from a subset
//! fall back along the chain. Text becomes SVG path data, so downstream
//! consumers (typst, rasterizers) need no font machinery at all.

use std::fmt::Write as _;

use ab_glyph::{Font, FontRef, GlyphId, OutlineCurve, PxScale, ScaleFont};

const EXCALIFONT: &[u8] = include_bytes!("../assets/Excalifont-Regular.ttf");
const VIRGIL: &[u8] = include_bytes!("../assets/Virgil.ttf");
const LIBERATION: &[u8] = include_bytes!("../assets/LiberationSans.ttf");
const CASCADIA: &[u8] = include_bytes!("../assets/Cascadia.ttf");

pub struct Fonts {
    excalifont: FontRef<'static>,
    virgil: FontRef<'static>,
    liberation: FontRef<'static>,
    cascadia: FontRef<'static>,
}

impl Default for Fonts {
    fn default() -> Self {
        Self::new()
    }
}

impl Fonts {
    pub fn new() -> Self {
        Self {
            excalifont: FontRef::try_from_slice(EXCALIFONT).expect("bundled Excalifont parses"),
            virgil: FontRef::try_from_slice(VIRGIL).expect("bundled Virgil parses"),
            liberation: FontRef::try_from_slice(LIBERATION).expect("bundled Liberation parses"),
            cascadia: FontRef::try_from_slice(CASCADIA).expect("bundled Cascadia parses"),
        }
    }

    /// Font chain for an Excalidraw `fontFamily` code: primary first,
    /// then cross-family fallbacks.
    fn chain(&self, family: i32) -> Vec<&FontRef<'static>> {
        match family {
            // 1 = legacy handwriting (Virgil)
            1 => vec![&self.virgil, &self.excalifont, &self.liberation],
            // 2 = "normal" (Helvetica-metric sans)
            2 => vec![&self.liberation, &self.excalifont],
            // 3 = code
            3 => vec![&self.cascadia, &self.liberation, &self.excalifont],
            // 4+ = Excalifont (current default) and anything unknown
            _ => vec![&self.excalifont, &self.virgil, &self.liberation],
        }
    }

    /// Laid-out line: its glyphs positioned in SVG coordinates, and its
    /// advance width. Outlines are pulled unscaled (`Font::outline`, font
    /// units, y-up) and scaled by `size / units_per_em` here, since
    /// `outline_glyph` keeps its curves private.
    fn layout_line(
        &self,
        line: &str,
        family: i32,
        size: f64,
        pen_x: f64,
        baseline_y: f64,
    ) -> (String, f64) {
        let scale = PxScale::from(size as f32);
        let mut d = String::new();
        let mut x = pen_x;
        let mut width = 0.0f64;
        let chain = self.chain(family);
        for ch in line.chars() {
            if ch == '\t' {
                x += size * 2.0;
                width += size * 2.0;
                continue;
            }
            // First font in the chain that has the glyph wins.
            let mut placed = false;
            for font in &chain {
                let id: GlyphId = font.glyph_id(ch);
                if id.0 == 0 {
                    continue; // .notdef — try the next family
                }
                let advance = font.as_scaled(scale).h_advance(id) as f64;
                if let Some(outline) = font.outline(id) {
                    let upem = font.units_per_em().unwrap_or(1000.0) as f64;
                    let unit_scale = (size / upem) as f32;
                    push_outline(&mut d, &outline.curves, x, baseline_y, unit_scale);
                }
                x += advance;
                width += advance;
                placed = true;
                break;
            }
            if !placed {
                // No font covers the codepoint: advance a rough space.
                x += size * 0.4;
                width += size * 0.4;
            }
        }
        (d, width)
    }

    /// Renders multi-line text as SVG path data anchored at `(x, y)`
    /// (top-left of the text box, y down), honoring horizontal alignment
    /// within `max_width`. Returns the path data and the total height.
    /// Word-wraps `text` to `max_width` using the family's metrics.
    /// Explicit `\n`s are respected; overlong words are hard-broken.
    pub fn wrap_text(&self, text: &str, family: i32, size: f64, max_width: f64) -> Vec<String> {
        let space_w = self.layout_line(" ", family, size, 0.0, 0.0).1;
        let mut out = Vec::new();
        for para in text.split('\n') {
            let mut line = String::new();
            let mut line_w = 0.0;
            for word in para.split(' ') {
                let word_w = self.layout_line(word, family, size, 0.0, 0.0).1;
                let add_w = if line.is_empty() {
                    word_w
                } else {
                    space_w + word_w
                };
                if !line.is_empty() && line_w + add_w > max_width {
                    out.push(std::mem::take(&mut line));
                    line_w = 0.0;
                    // The word itself may exceed the width: hard-break it.
                    if word_w > max_width && max_width > 0.0 {
                        let mut chunk = String::new();
                        let mut chunk_w = 0.0;
                        for ch in word.chars() {
                            let ch_w = self.layout_line(&ch.to_string(), family, size, 0.0, 0.0).1;
                            if chunk_w + ch_w > max_width && !chunk.is_empty() {
                                out.push(std::mem::take(&mut chunk));
                                chunk_w = 0.0;
                            }
                            chunk.push(ch);
                            chunk_w += ch_w;
                        }
                        line = chunk;
                        line_w = chunk_w;
                        continue;
                    }
                }
                if !line.is_empty() {
                    line.push(' ');
                    line_w += space_w;
                }
                line.push_str(word);
                line_w += word_w;
            }
            out.push(line);
        }
        out
    }

    /// Total rendered height of the (possibly wrapped) text block.
    pub fn text_height(
        &self,
        text: &str,
        family: i32,
        size: f64,
        line_height: f64,
        wrap_width: Option<f64>,
    ) -> f64 {
        let lines = match wrap_width {
            Some(w) => self.wrap_text(text, family, size, w),
            None => text.split('\n').map(str::to_string).collect(),
        };
        lines.len() as f64 * size * line_height
    }

    #[allow(clippy::too_many_arguments)] // font layout parameters travel together
    pub fn text_to_path(
        &self,
        text: &str,
        family: i32,
        size: f64,
        line_height: f64,
        x: f64,
        y: f64,
        align: &str,
        max_width: f64,
        wrap: bool,
    ) -> (String, f64) {
        let primary = self.chain(family)[0];
        let scale = PxScale::from(size as f32);
        let v = primary.as_scaled(scale);
        let ascent = v.ascent() as f64;
        let line_gap = v.line_gap() as f64;
        let line_h = size * line_height;

        let lines: Vec<String> = if wrap {
            self.wrap_text(text, family, size, max_width)
        } else {
            text.split('\n').map(str::to_string).collect()
        };
        // Pass 1: widths for alignment.
        let widths: Vec<f64> = lines
            .iter()
            .map(|l| self.layout_line(l, family, size, 0.0, 0.0).1)
            .collect();

        let mut d = String::new();
        for (i, line) in lines.iter().enumerate() {
            if line.is_empty() {
                continue;
            }
            let line_w = widths[i];
            let pen_x = match align {
                "center" => x + (max_width - line_w) / 2.0,
                "right" => x + (max_width - line_w),
                _ => x,
            };
            let baseline = y + i as f64 * line_h + ascent + line_gap;
            let (line_d, _) = self.layout_line(line, family, size, pen_x, baseline);
            d.push_str(&line_d);
        }
        (d, lines.len() as f64 * line_h)
    }
}

/// Appends glyph outline curves (font units, y-up) into `d` as SVG path
/// data, scaled by `scale` and flipped around the baseline. Each curve
/// carries its start point; the first starts a subpath.
fn push_outline(d: &mut String, curves: &[OutlineCurve], pen_x: f64, baseline_y: f64, scale: f32) {
    let to_svg = |p: &ab_glyph::Point| {
        format!(
            "{:.2},{:.2} ",
            pen_x + p.x as f64 * scale as f64,
            baseline_y - p.y as f64 * scale as f64
        )
    };
    let mut first = true;
    for curve in curves {
        match curve {
            OutlineCurve::Line(p0, p1) => {
                if first {
                    let _ = write!(d, "M{} ", to_svg(p0));
                    first = false;
                }
                let _ = write!(d, "L{} ", to_svg(p1));
            }
            OutlineCurve::Quad(p0, q, p1) => {
                if first {
                    let _ = write!(d, "M{} ", to_svg(p0));
                    first = false;
                }
                let _ = write!(d, "Q{}{} ", to_svg(q), to_svg(p1));
            }
            OutlineCurve::Cubic(p0, c1, c2, p1) => {
                if first {
                    let _ = write!(d, "M{} ", to_svg(p0));
                    first = false;
                }
                let _ = write!(d, "C{}{}{} ", to_svg(c1), to_svg(c2), to_svg(p1));
            }
        }
    }
    if !first {
        d.push_str("Z ");
    }
}
