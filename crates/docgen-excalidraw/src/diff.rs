//! Element-id structural diff between two scenes: added / removed /
//! changed elements, with the changed *fields* enumerated per element.
//! Volatile bookkeeping fields (version counters, update timestamps,
//! versionNonce) are ignored so re-saving a scene in the app is not a
//! change; a different `seed` counts (it changes the hand-drawn look).

use std::collections::BTreeMap;

use crate::model::{Element, Scene};

/// Fields that change on every save or edit without changing the drawing.
const VOLATILE: &[&str] = &["version", "versionNonce", "updated"];

#[derive(Debug, Clone, Default, PartialEq)]
pub struct SceneDiff {
    pub added: Vec<String>,
    pub removed: Vec<String>,
    pub changed: Vec<ElementChange>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ElementChange {
    pub id: String,
    pub kind: String,
    pub fields: Vec<String>,
}

impl SceneDiff {
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty() && self.changed.is_empty()
    }

    /// One-line human summary for review comments, e.g.
    /// "2 elements added, 1 removed, 1 changed (text)".
    pub fn summary(&self) -> String {
        if self.is_empty() {
            return "no element changes".to_string();
        }
        let mut parts = Vec::new();
        let n = |v: &[String]| v.len();
        if !self.added.is_empty() {
            parts.push(format!("{} added", n(&self.added)));
        }
        if !self.removed.is_empty() {
            parts.push(format!("{} removed", n(&self.removed)));
        }
        if !self.changed.is_empty() {
            parts.push(format!("{} changed", self.changed.len()));
        }
        parts.join(", ")
    }
}

/// Diffs two scenes by element id. Deleted-but-present elements
/// (`isDeleted`) are ignored on both sides.
pub fn diff_scenes(old: &Scene, new: &Scene) -> SceneDiff {
    let old_map: BTreeMap<&str, &Element> = old
        .elements
        .iter()
        .filter(|e| !e.is_deleted)
        .map(|e| (e.id.as_str(), e))
        .collect();
    let new_map: BTreeMap<&str, &Element> = new
        .elements
        .iter()
        .filter(|e| !e.is_deleted)
        .map(|e| (e.id.as_str(), e))
        .collect();

    let mut diff = SceneDiff::default();
    for (id, ne) in &new_map {
        match old_map.get(*id) {
            None => diff.added.push((*id).to_string()),
            Some(oe) => {
                let fields = changed_fields(oe, ne);
                if !fields.is_empty() {
                    diff.changed.push(ElementChange {
                        id: (*id).to_string(),
                        kind: ne.kind.clone(),
                        fields,
                    });
                }
            }
        }
    }
    for id in old_map.keys() {
        if !new_map.contains_key(id) {
            diff.removed.push((*id).to_string());
        }
    }
    diff
}

/// Names of render-relevant fields that differ between two revisions of
/// the same element.
pub fn changed_fields(old: &Element, new: &Element) -> Vec<String> {
    let mut out = Vec::new();
    macro_rules! field {
        ($name:literal, $a:expr, $b:expr) => {
            if $a != $b {
                out.push($name.to_string());
            }
        };
    }
    field!("type", old.kind, new.kind);
    field!("x", old.x, new.x);
    field!("y", old.y, new.y);
    field!("width", old.width, new.width);
    field!("height", old.height, new.height);
    field!("angle", old.angle, new.angle);
    field!("strokeColor", old.stroke_color, new.stroke_color);
    field!(
        "backgroundColor",
        old.background_color,
        new.background_color
    );
    field!("fillStyle", old.fill_style, new.fill_style);
    field!("strokeWidth", old.stroke_width, new.stroke_width);
    field!("strokeStyle", old.stroke_style, new.stroke_style);
    field!("roughness", old.roughness, new.roughness);
    field!("opacity", old.opacity, new.opacity);
    field!("seed", old.seed, new.seed);
    field!("points", old.points, new.points);
    field!("text", old.text, new.text);
    field!("fontSize", old.font_size, new.font_size);
    field!("fontFamily", old.font_family, new.font_family);
    field!("textAlign", old.text_align, new.text_align);
    field!("containerId", old.container_id, new.container_id);
    field!("fileId", old.file_id, new.file_id);

    let mut old_extra: Vec<(&String, &serde_json::Value)> = old
        .extra
        .iter()
        .filter(|(k, _)| !VOLATILE.contains(&k.as_str()))
        .collect();
    old_extra.sort_by(|a, b| a.0.cmp(b.0));
    let mut new_extra: Vec<(&String, &serde_json::Value)> = new
        .extra
        .iter()
        .filter(|(k, _)| !VOLATILE.contains(&k.as_str()))
        .collect();
    new_extra.sort_by(|a, b| a.0.cmp(b.0));
    field!("metadata", old_extra, new_extra);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn el(id: &str, kind: &str, x: f64, y: f64) -> Element {
        Element {
            kind: kind.to_string(),
            id: id.to_string(),
            x,
            y,
            ..Default::default()
        }
    }

    impl Default for Element {
        fn default() -> Self {
            serde_json::from_value(serde_json::json!({
                "type": "rectangle", "id": "x"
            }))
            .unwrap()
        }
    }

    fn scene(elements: Vec<Element>) -> Scene {
        Scene {
            elements,
            ..Default::default()
        }
    }

    #[test]
    fn classifies_add_remove_change() {
        let old = scene(vec![
            el("keep", "rectangle", 0.0, 0.0),
            el("gone", "ellipse", 10.0, 10.0),
            el("moved", "text", 5.0, 5.0),
        ]);
        let mut moved = el("moved", "text", 50.0, 5.0);
        moved.text = "edited".into();
        let new = scene(vec![
            el("keep", "rectangle", 0.0, 0.0),
            moved,
            el("fresh", "arrow", 20.0, 20.0),
        ]);

        let d = diff_scenes(&old, &new);
        assert_eq!(d.added, vec!["fresh"]);
        assert_eq!(d.removed, vec!["gone"]);
        assert_eq!(d.changed.len(), 1);
        assert_eq!(d.changed[0].id, "moved");
        assert!(d.changed[0].fields.contains(&"x".to_string()));
        assert!(d.changed[0].fields.contains(&"text".to_string()));
        assert_eq!(d.summary(), "1 added, 1 removed, 1 changed");
    }

    #[test]
    fn volatile_fields_are_not_changes() {
        let old = scene(vec![el("a", "rectangle", 0.0, 0.0)]);
        let mut e = el("a", "rectangle", 0.0, 0.0);
        e.extra.insert("version".into(), serde_json::json!(2));
        e.extra
            .insert("versionNonce".into(), serde_json::json!(999));
        e.extra
            .insert("updated".into(), serde_json::json!(1700000000));
        let new = scene(vec![e]);
        assert!(diff_scenes(&old, &new).is_empty());
    }

    #[test]
    fn deleted_elements_are_ignored() {
        let mut dying = el("d", "rectangle", 0.0, 0.0);
        dying.is_deleted = true;
        let old = scene(vec![dying]);
        let new = scene(vec![]);
        assert!(diff_scenes(&old, &new).is_empty());
    }
}
