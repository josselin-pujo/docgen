//! YAML frontmatter splitting and typing.
//!
//! Deliberately hand-rolled (a leading `---` fenced YAML block, closed by a
//! `---` or `...` line) so the crate stays free of I/O-shaped dependencies.
//! Handles LF and CRLF, an optional BOM, and an optional empty first line.

use crate::{Error, Frontmatter, Result};
use serde_yaml_ng;

/// Splits a markdown document into `(yaml_frontmatter, body)`.
/// `yaml` is the raw YAML between the delimiters (unparsed).
pub fn split(markdown: &str) -> (Option<&str>, &str) {
    let markdown = markdown.strip_prefix('\u{feff}').unwrap_or(markdown);
    let mut lines = markdown.split_inclusive('\n');

    let first = match lines.next() {
        Some(l) => l,
        None => return (None, markdown),
    };
    if !is_fence_line(first) {
        return (None, markdown);
    }

    let mut consumed = first.len();
    for line in lines {
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed == "---" || trimmed == "..." {
            let yaml = &markdown[first.len()..consumed];
            let body = &markdown[consumed + line.len()..];
            return (Some(yaml), body);
        }
        consumed += line.len();
    }
    // Unterminated frontmatter: treat the whole input as body.
    (None, markdown)
}

fn is_fence_line(line: &str) -> bool {
    line.trim_end_matches(['\r', '\n']) == "---"
}

/// Parses raw YAML into the typed [`Frontmatter`], preserving unknown keys.
pub fn parse(yaml: &str) -> Result<Frontmatter> {
    if yaml.trim().is_empty() {
        return Ok(Frontmatter::default());
    }
    let mut fm: Frontmatter = serde_yaml_ng::from_str(yaml)
        .map_err(|e| Error::Frontmatter(format!("YAML error: {e}")))?;
    // YAML dates like `2026-10-03` may deserialize as mappings/whatnot for
    // String fields via serde_json bridging; normalize the common scalars.
    if let Some(v) = fm.extra.remove("date")
        && fm.date.is_none()
    {
        fm.date = scalar_to_string(&v);
    }
    // The `media:` map is read typed ([`Frontmatter::media`]); reject
    // malformed shapes here so typos fail loud instead of silently
    // falling back to defaults.
    if let Some(v) = fm.extra.get("media")
        && !v.is_null()
    {
        serde_json::from_value::<crate::model::MediaConfig>(v.clone()).map_err(|e| {
            Error::Frontmatter(format!("invalid `media:` map (expected placement: inline|appendix, index: bool): {e}"))
        })?;
    }
    Ok(fm)
}

fn scalar_to_string(v: &serde_json::Value) -> Option<String> {
    match v {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Number(n) => Some(n.to_string()),
        serde_json::Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_simple() {
        let md = "---\ntitle: Hi\n---\n\nbody";
        let (yaml, body) = split(md);
        assert_eq!(yaml.unwrap().trim(), "title: Hi");
        assert_eq!(body, "\nbody");
    }

    #[test]
    fn handles_crlf() {
        let md = "---\r\ntitle: Hi\r\n---\r\nbody\r\n";
        let (yaml, body) = split(md);
        assert!(yaml.unwrap().contains("title"));
        assert_eq!(body, "body\r\n");
    }

    #[test]
    fn no_frontmatter() {
        let (yaml, body) = split("# just a doc\n");
        assert!(yaml.is_none());
        assert_eq!(body, "# just a doc\n");
    }

    #[test]
    fn unterminated_is_body() {
        let (yaml, _) = split("---\ntitle: Hi\n");
        assert!(yaml.is_none());
    }

    #[test]
    fn parses_typed_fields() {
        let fm = parse("title: T\nauthors: [a, b]\nunknown: 42\nversion: 1.2\n").unwrap();
        assert_eq!(fm.title.as_deref(), Some("T"));
        assert_eq!(fm.authors, ["a", "b"]);
        assert_eq!(fm.extra.get("unknown").and_then(|v| v.as_i64()), Some(42));
        assert_eq!(fm.version.as_deref(), Some("1.2"));
    }

    #[test]
    fn media_defaults_when_absent() {
        let fm = parse("title: T\n").unwrap();
        assert_eq!(fm.media(), crate::model::MediaConfig::default());
        assert_eq!(fm.media().placement, crate::model::Placement::Inline);
        assert!(!fm.media().index);
    }

    #[test]
    fn media_explicit_values_and_partial_maps() {
        let fm = parse("media:\n  placement: appendix\n  index: true\n").unwrap();
        assert_eq!(fm.media().placement, crate::model::Placement::Appendix);
        assert!(fm.media().index);
        // The map stays in extra (renderer meta passthrough).
        assert!(fm.extra.contains_key("media"));

        let fm = parse("media:\n  index: true\n").unwrap();
        assert_eq!(fm.media().placement, crate::model::Placement::Inline);
        assert!(fm.media().index);
    }

    #[test]
    fn media_null_map_means_defaults() {
        let fm = parse("media:\n").unwrap();
        assert_eq!(fm.media(), crate::model::MediaConfig::default());
    }

    #[test]
    fn media_malformed_map_is_an_error() {
        assert!(parse("media:\n  placement: inlane\n").is_err());
        assert!(parse("media: 5\n").is_err());
    }
}
