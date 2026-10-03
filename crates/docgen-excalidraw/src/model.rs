//! Serde model for the `.excalidraw` scene format.
//!
//! Deliberately permissive: elements are one flat struct with optional
//! fields (the format adds fields faster than any schema), and unknown
//! fields are preserved through [`Scene::extra`] so round-tripping never
//! loses data. [`Element::id`] is the join key for structural diffing and
//! for merging externally-edited scenes.

use serde::{Deserialize, Serialize};

/// A parsed `.excalidraw` document.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Scene {
    #[serde(default)]
    pub elements: Vec<Element>,
    #[serde(rename = "appState", default)]
    pub app_state: AppState,
    #[serde(default)]
    pub files: std::collections::BTreeMap<String, SceneFile>,
    /// `type`, `source`, `version`, and any other top-level keys.
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppState {
    #[serde(rename = "viewBackgroundColor", default = "default_background")]
    pub view_background_color: String,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            view_background_color: default_background(),
            extra: serde_json::Map::new(),
        }
    }
}

fn default_background() -> String {
    "#ffffff".to_string()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SceneFile {
    #[serde(rename = "mimeType", default)]
    pub mime_type: String,
    #[serde(default)]
    pub id: String,
    #[serde(rename = "dataURL", default)]
    pub data_url: String,
}

/// One scene element. `kind` is the `"type"` discriminator; per-type
/// fields are optional and only meaningful for their kinds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Element {
    #[serde(rename = "type")]
    pub kind: String,
    pub id: String,
    #[serde(default)]
    pub x: f64,
    #[serde(default)]
    pub y: f64,
    #[serde(default)]
    pub width: f64,
    #[serde(default)]
    pub height: f64,
    /// Radians, around the element center.
    #[serde(default)]
    pub angle: f64,
    #[serde(rename = "strokeColor", default = "default_stroke")]
    pub stroke_color: String,
    #[serde(rename = "backgroundColor", default)]
    pub background_color: String,
    #[serde(rename = "fillStyle", default = "default_fill_style")]
    pub fill_style: String,
    #[serde(rename = "strokeWidth", default = "default_stroke_width")]
    pub stroke_width: f64,
    #[serde(rename = "strokeStyle", default = "default_stroke_style")]
    pub stroke_style: String,
    #[serde(default = "default_roughness")]
    pub roughness: f64,
    /// 0..=100.
    #[serde(default = "default_opacity")]
    pub opacity: f64,
    #[serde(default)]
    pub seed: i64,
    #[serde(rename = "isDeleted", default)]
    pub is_deleted: bool,
    #[serde(default)]
    pub locked: bool,
    #[serde(default)]
    pub group_ids: Vec<String>,
    #[serde(default)]
    pub roundness: Option<serde_json::Value>,
    // -- shape/line specifics ------------------------------------------------
    /// Relative to (x, y); arrows/lines/draw.
    #[serde(default)]
    pub points: Vec<(f64, f64)>,
    #[serde(rename = "startArrowhead", default)]
    pub start_arrowhead: Option<String>,
    #[serde(rename = "endArrowhead", default)]
    pub end_arrowhead: Option<String>,
    #[serde(rename = "lastCommittedPoint", default)]
    pub last_committed_point: Option<(f64, f64)>,
    // -- text specifics ------------------------------------------------------
    #[serde(default)]
    pub text: String,
    #[serde(default = "default_font_size")]
    pub font_size: f64,
    #[serde(rename = "fontFamily", default = "default_font_family")]
    pub font_family: i32,
    #[serde(rename = "textAlign", default = "default_text_align")]
    pub text_align: String,
    #[serde(rename = "verticalAlign", default)]
    pub vertical_align: String,
    #[serde(default = "default_line_height")]
    pub line_height: f64,
    #[serde(rename = "containerId", default)]
    pub container_id: Option<String>,
    #[serde(rename = "originalText", default)]
    pub original_text: Option<String>,
    // -- image specifics -----------------------------------------------------
    #[serde(rename = "fileId", default)]
    pub file_id: Option<String>,
    #[serde(rename = "scale", default = "default_image_scale")]
    pub image_scale: Option<(f64, f64)>,
    /// Everything else (version, versionNonce, updated, boundElements,
    /// bindings, pressures, ...).
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

fn default_stroke() -> String {
    "#1e1e1e".to_string()
}
fn default_fill_style() -> String {
    "hachure".to_string()
}
fn default_stroke_width() -> f64 {
    1.0
}
fn default_stroke_style() -> String {
    "solid".to_string()
}
fn default_roughness() -> f64 {
    1.0
}
fn default_opacity() -> f64 {
    100.0
}
fn default_font_size() -> f64 {
    20.0
}
fn default_font_family() -> i32 {
    1
}
fn default_text_align() -> String {
    "left".to_string()
}
fn default_line_height() -> f64 {
    1.25
}
fn default_image_scale() -> Option<(f64, f64)> {
    Some((1.0, 1.0))
}

impl Element {
    pub fn is_shape(&self) -> bool {
        matches!(self.kind.as_str(), "rectangle" | "diamond" | "ellipse")
    }

    pub fn is_line_like(&self) -> bool {
        matches!(self.kind.as_str(), "arrow" | "line" | "draw")
    }

    /// Absolute endpoint of the last point (arrows/lines).
    pub fn abs_points(&self) -> Vec<(f64, f64)> {
        if self.points.is_empty() {
            return vec![(self.x, self.y), (self.x + self.width, self.y)];
        }
        self.points
            .iter()
            .map(|(px, py)| (self.x + px, self.y + py))
            .collect()
    }

    /// Center of the bounding box — rotation pivot.
    pub fn center(&self) -> (f64, f64) {
        (self.x + self.width / 2.0, self.y + self.height / 2.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: &str = r##"{
      "type": "excalidraw",
      "version": 2,
      "elements": [
        {"type": "rectangle", "id": "r1", "x": 10, "y": 20, "width": 100, "height": 60,
         "strokeColor": "#e03131", "backgroundColor": "#a5d8ff", "fillStyle": "solid",
         "strokeWidth": 2, "roughness": 1, "opacity": 100, "seed": 42, "angle": 0,
         "version": 1, "versionNonce": 1, "isDeleted": false},
        {"type": "arrow", "id": "a1", "x": 120, "y": 50, "width": 80, "height": 0,
         "points": [[0,0],[80,0]], "strokeColor": "#1e1e1e", "strokeWidth": 1,
         "roughness": 1, "opacity": 100, "seed": 7, "endArrowhead": "arrow",
         "startArrowhead": null, "fillStyle": "hachure", "version": 1,
         "versionNonce": 2, "isDeleted": false},
        {"type": "text", "id": "t1", "x": 30, "y": 35, "width": 60, "height": 25,
         "text": "hello\nworld", "fontSize": 20, "fontFamily": 1,
         "textAlign": "center", "verticalAlign": "middle", "containerId": "r1",
         "strokeColor": "#1e1e1e", "backgroundColor": "transparent",
         "fillStyle": "hachure", "strokeWidth": 1, "roughness": 1,
         "opacity": 100, "seed": 9, "angle": 0, "version": 1,
         "versionNonce": 3, "isDeleted": false}
      ],
      "appState": {"viewBackgroundColor": "#ffffff"},
      "files": {}
    }"##;

    #[test]
    fn parses_minimal_scene() {
        let scene: Scene = serde_json::from_str(MIN).unwrap();
        assert_eq!(scene.elements.len(), 3);
        assert_eq!(scene.elements[0].id, "r1");
        assert_eq!(scene.elements[0].background_color, "#a5d8ff");
        assert_eq!(scene.elements[1].end_arrowhead.as_deref(), Some("arrow"));
        assert_eq!(scene.elements[2].text, "hello\nworld");
        assert_eq!(scene.elements[2].container_id.as_deref(), Some("r1"));
        assert_eq!(scene.app_state.view_background_color, "#ffffff");
        // top-level extras preserved
        assert_eq!(scene.extra.get("type").and_then(|v| v.as_str()), Some("excalidraw"));
    }

    #[test]
    fn roundtrips_without_loss() {
        let scene: Scene = serde_json::from_str(MIN).unwrap();
        let out = serde_json::to_string(&scene).unwrap();
        let back: Scene = serde_json::from_str(&out).unwrap();
        assert_eq!(scene, back);
    }

    #[test]
    fn tolerates_unknown_fields_and_missing_defaults() {
        let scene: Scene = serde_json::from_str(
            r#"{"elements": [{"type": "line", "id": "l1", "x": 0, "y": 0, "futureField": 1}], "brandNewTop": true}"#,
        )
        .unwrap();
        assert_eq!(scene.elements[0].stroke_width, 1.0);
        assert_eq!(scene.elements[0].extra.get("futureField").and_then(|v| v.as_i64()), Some(1));
    }
}
