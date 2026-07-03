//! Note rendering and managed-region surgery.
//!
//! A fresh note is:
//!
//! ```markdown
//! ---
//! schema: curio.frontmatter.v1
//! …machine keys…
//! ---
//!
//! <!-- curio:managed:begin v1 -->
//! …extracted article markdown…
//! <!-- curio:managed:end -->
//! ```
//!
//! On re-export, only the frontmatter *machine keys* and the bytes
//! between the markers change. Everything else — text between the
//! frontmatter and the begin marker, everything after the end marker,
//! and frontmatter keys Curio does not know — is preserved exactly.
//! The checksum covers the bytes strictly between the begin-marker line
//! and the end-marker line (excluding the delimiting newlines).

use std::path::Path;

use curio_types::{ArticleFrontmatter, MANAGED_REGION_BEGIN_V1, MANAGED_REGION_END_V1};

use super::ExportError;

/// The three preserved spans of an existing note around Curio's two
/// owned surfaces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteParts {
    /// The raw YAML between the `---` fences (without them).
    pub yaml: String,
    /// Bytes between the frontmatter's closing fence and the begin
    /// marker — user-owned, preserved byte-for-byte.
    pub before_region: String,
    /// The current managed-region body.
    pub region: String,
    /// Bytes after the end marker — user-owned, preserved byte-for-byte.
    pub after_region: String,
}

/// Renders a fresh note.
///
/// # Errors
///
/// [`ExportError::Yaml`] if the frontmatter fails to serialize.
pub fn render_note(frontmatter: &ArticleFrontmatter, body: &str) -> Result<String, ExportError> {
    let yaml = serde_yaml::to_string(frontmatter)?;
    Ok(assemble(&yaml, "\n", body, "\n"))
}

/// Splits an existing note into its preserved and managed parts.
///
/// # Errors
///
/// [`ExportError::NoteParse`] when the frontmatter fences or the
/// managed-region markers are missing or out of order — Curio refuses
/// to rewrite a file it cannot account for byte-by-byte.
pub fn split_note(content: &str, path: &Path) -> Result<NoteParts, ExportError> {
    let parse_err = |message: &str| ExportError::NoteParse {
        path: path.to_path_buf(),
        message: message.to_owned(),
    };
    let rest = content
        .strip_prefix("---\n")
        .ok_or_else(|| parse_err("missing opening frontmatter fence"))?;
    let fence_end = rest
        .find("\n---\n")
        .ok_or_else(|| parse_err("missing closing frontmatter fence"))?;
    let yaml = &rest[..=fence_end];
    let after_fm = &rest[fence_end + "\n---\n".len()..];

    let begin = after_fm
        .find(MANAGED_REGION_BEGIN_V1)
        .ok_or_else(|| parse_err("missing managed-region begin marker"))?;
    let region_start = begin + MANAGED_REGION_BEGIN_V1.len();
    let end_rel = after_fm[region_start..]
        .find(MANAGED_REGION_END_V1)
        .ok_or_else(|| parse_err("missing managed-region end marker"))?;
    let region_end = region_start + end_rel;

    let region = after_fm[region_start..region_end]
        .strip_prefix('\n')
        .unwrap_or(&after_fm[region_start..region_end]);
    let region = region.strip_suffix('\n').unwrap_or(region);

    Ok(NoteParts {
        yaml: yaml.to_owned(),
        before_region: after_fm[..begin].to_owned(),
        region: region.to_owned(),
        after_region: after_fm[region_end + MANAGED_REGION_END_V1.len()..].to_owned(),
    })
}

/// Rewrites an existing note: new machine keys + new region body, user
/// bytes and unknown frontmatter keys preserved.
///
/// # Errors
///
/// [`ExportError::NoteParse`] for an unaccountable note,
/// [`ExportError::Yaml`] on frontmatter (de)serialization failures.
pub fn replace_managed(
    existing: &str,
    fresh: &ArticleFrontmatter,
    body: &str,
    path: &Path,
) -> Result<String, ExportError> {
    let parts = split_note(existing, path)?;
    let old: ArticleFrontmatter =
        serde_yaml::from_str(&parts.yaml).map_err(|err| ExportError::NoteParse {
            path: path.to_path_buf(),
            message: format!("frontmatter does not parse as curio.frontmatter.v1: {err}"),
        })?;
    let mut merged = fresh.clone();
    merged.extra = old.extra; // unknown keys are the user's — keep them
    let yaml = serde_yaml::to_string(&merged)?;
    Ok(assemble(
        &yaml,
        &parts.before_region,
        body,
        &parts.after_region,
    ))
}

fn assemble(yaml: &str, before: &str, body: &str, after: &str) -> String {
    let mut out = String::with_capacity(yaml.len() + before.len() + body.len() + after.len() + 96);
    out.push_str("---\n");
    out.push_str(yaml);
    if !yaml.ends_with('\n') {
        out.push('\n');
    }
    out.push_str("---\n");
    out.push_str(before);
    out.push_str(MANAGED_REGION_BEGIN_V1);
    out.push('\n');
    out.push_str(body);
    out.push('\n');
    out.push_str(MANAGED_REGION_END_V1);
    out.push_str(after);
    out
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use std::collections::BTreeMap;

    use curio_types::FrontmatterSchemaV1Marker;

    use super::super::region_checksum;
    use super::*;

    fn sample_frontmatter(title: &str) -> ArticleFrontmatter {
        ArticleFrontmatter {
            schema: FrontmatterSchemaV1Marker,
            curio_id: "0197b2c4-8f3e-7cc1-a5d2-3e9f10aa4b6d".parse().unwrap(),
            title: title.to_owned(),
            source: "https://example.com/article".to_owned(),
            feed: Some("https://example.com/feed.xml".to_owned()),
            feed_title: Some("Example Blog".to_owned()),
            author: None,
            published: None,
            saved: "2026-07-03T09:15:00.123Z".parse().unwrap(),
            tags: vec!["rust".to_owned()],
            checksum: region_checksum("body"),
            lang: None,
            word_count: Some(2),
            extra: BTreeMap::new(),
        }
    }

    #[test]
    fn fresh_note_round_trips_through_split() {
        let note = render_note(&sample_frontmatter("T"), "line one\n\nline two").unwrap();
        assert!(note.starts_with("---\nschema: curio.frontmatter.v1\n"));
        assert!(note.contains(MANAGED_REGION_BEGIN_V1));
        assert!(note.ends_with("<!-- curio:managed:end -->\n"));

        let parts = split_note(&note, Path::new("t.md")).unwrap();
        assert_eq!(parts.region, "line one\n\nline two");
        assert_eq!(parts.before_region, "\n");
        assert_eq!(parts.after_region, "\n");
        let fm: ArticleFrontmatter = serde_yaml::from_str(&parts.yaml).unwrap();
        assert_eq!(fm.title, "T");
    }

    #[test]
    fn replace_preserves_user_bytes_and_unknown_keys() {
        let note = render_note(&sample_frontmatter("Old title"), "old body").unwrap();
        // The user annotates: an unknown frontmatter key, a note above
        // the region, and companion text below it.
        let edited = note
            .replace(
                "---\n\n<!-- curio:managed:begin",
                "kp_status: enriched\nrating: 5\n---\nUser preamble.\n\n<!-- curio:managed:begin",
            )
            .replace(
                "<!-- curio:managed:end -->\n",
                "<!-- curio:managed:end -->\n\n## My notes\n\nHand-written companion text.\n",
            );

        let fresh = ArticleFrontmatter {
            title: "New title".to_owned(),
            checksum: region_checksum("new body"),
            ..sample_frontmatter("New title")
        };
        let rewritten = replace_managed(&edited, &fresh, "new body", Path::new("t.md")).unwrap();

        assert!(rewritten.contains("title: New title"));
        assert!(rewritten.contains("new body"));
        assert!(!rewritten.contains("old body"));
        assert!(rewritten.contains("kp_status: enriched"));
        assert!(rewritten.contains("rating: 5"));
        assert!(rewritten.contains("User preamble.\n\n<!-- curio:managed:begin"));
        assert!(
            rewritten.ends_with(
                "<!-- curio:managed:end -->\n\n## My notes\n\nHand-written companion text.\n"
            ),
            "companion text must survive byte-for-byte:\n{rewritten}"
        );
    }

    #[test]
    fn split_refuses_notes_without_markers() {
        let err = split_note("---\ntitle: x\n---\nno markers here\n", Path::new("t.md"));
        assert!(matches!(err, Err(ExportError::NoteParse { .. })));
        let err = split_note("no frontmatter at all", Path::new("t.md"));
        assert!(matches!(err, Err(ExportError::NoteParse { .. })));
    }
}
