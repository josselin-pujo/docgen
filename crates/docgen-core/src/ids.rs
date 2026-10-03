//! Stable block identity.
//!
//! Diffing (and asset bookkeeping) hinges on block ids that do not move when
//! unrelated parts of the document change:
//! - explicit ids (`{#my-id}` heading attribute, ```` ```mermaid {#my-id} ````
//!   info attribute) always win;
//! - headings fall back to a slug of their text;
//! - everything else falls back to a content hash (fnv-1a 64), prefixed by
//!   block kind, so identical content keeps its id and an edit is seen as a
//!   replace of exactly that block.

/// FNV-1a 64-bit, hex-encoded. Deterministic, dependency-free, stable across
/// runs and platforms — exactly what content hashing for ids needs.
pub fn fnv1a64(data: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in data.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

/// URL-ish slug for heading text: lowercase ASCII, runs of non-alphanumerics
/// collapsed to single dashes.
pub fn slug(text: &str) -> String {
    let mut slug = String::with_capacity(text.len());
    let mut pending_dash = false;
    for ch in text.chars() {
        let lower = ch.to_ascii_lowercase();
        if lower.is_ascii_alphanumeric() {
            if pending_dash && !slug.is_empty() {
                slug.push('-');
            }
            pending_dash = false;
            slug.push(lower);
        } else if !slug.is_empty() {
            pending_dash = true;
        }
    }
    if slug.is_empty() {
        slug.push_str("section");
    }
    slug
}

/// Assigns non-conflicting ids by suffixing `-2`, `-3`, ... on collisions.
#[derive(Debug, Default)]
pub struct IdAllocator {
    used: std::collections::HashSet<String>,
}

impl IdAllocator {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn allocate(&mut self, wanted: String) -> String {
        if self.used.insert(wanted.clone()) {
            return wanted;
        }
        for n in 2.. {
            let candidate = format!("{wanted}-{n}");
            if self.used.insert(candidate.clone()) {
                return candidate;
            }
        }
        unreachable!("hash set grows without bound")
    }

    pub fn reserve(&mut self, id: &str) {
        self.used.insert(id.to_owned());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fnv_is_stable() {
        assert_eq!(fnv1a64("hello"), fnv1a64("hello"));
        assert_ne!(fnv1a64("hello"), fnv1a64("hellp"));
    }

    #[test]
    fn slugs() {
        assert_eq!(slug("Hello, World!"), "hello-world");
        assert_eq!(slug("  Étude  "), "tude"); // non-ASCII dropped, per doc
        assert_eq!(slug("???"), "section");
    }
}
