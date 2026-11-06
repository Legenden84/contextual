//! Builds what is sent to the model (`lib/prompt.ts`): rules + output format as the system
//! prompt, and each new user turn with only the files the model doesn't have yet (new, changed,
//! or "Resend all files"). Pictures go as image parts of the user turn.

use serde::{Deserialize, Serialize};

use super::lang::{Lang, lang_from_path};
use super::types::Rule;

/// A file attached to a turn.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextFile {
    pub path: String,
    /// `None` when the contents aren't available to the app.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    /// Image files: data URL sent as a picture.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    /// A condensed copy of a large HTML/SVG file: a read-only reference.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub condensed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextImage {
    pub name: String,
    pub url: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ImageDetail {
    High,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImageUrl {
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<ImageDetail>,
}

/// One part of a multi-part message (`ContentPart` in `lib/api.ts`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentPart {
    Text { text: String },
    ImageUrl { image_url: ImageUrl },
}

/// Text, or (user messages) text and pictures (`ApiContent`): a JSON string or an array of parts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ApiContent {
    Text(String),
    Parts(Vec<ContentPart>),
}

impl From<String> for ApiContent {
    fn from(s: String) -> Self {
        Self::Text(s)
    }
}

/// Header note for condensed references; [`super::changes`] never turns these into changes.
pub const REFERENCE_NOTE: &str = "(condensed reference, read-only: do not return this file)";

/// How the model must return file changes. Built in and not shown in Settings: existing files come
/// back as small edit blocks, which are rebuilt into whole files locally ([`super::edits`]).
pub const FILE_FORMAT: &str = "# How to return file changes

These rules take precedence over any earlier instruction about how to return files. Never write file contents outside the blocks below.

## Changing an existing file
Return only the parts that change, as one or more edit blocks after a header line:

### EDIT: src/path/to/File.cs
<<<<<<< SEARCH
exact existing lines, copied verbatim with their indentation
=======
the lines that replace them
>>>>>>> REPLACE

- SEARCH must match the current file exactly, character for character, including indentation, comments and blank lines.
- Include 2–3 unchanged lines around the change so SEARCH occurs only once in the file. Keep blocks small; never repeat a whole file in SEARCH.
- To insert lines, SEARCH for the lines next to the insertion point and repeat them in REPLACE together with the new lines.
- To remove lines, leave them out of REPLACE.
- Use several blocks for separate changes in the same file, in file order, without overlapping. One header per file.
- Do not put edit blocks in a code fence.

## New files, or rewriting most of a small file
### FILE: src/path/to/NewFile.cs
```csharp
complete file contents
```

## Deleting a file
### DELETE: src/path/to/Old.cs

Never return a file marked as a condensed reference.";

fn fence_lang(path: &str) -> &'static str {
    match lang_from_path(Some(path)) {
        Lang::Csharp => "csharp",
        Lang::Typescript => "typescript",
        Lang::Python => "python",
        Lang::Json => "json",
        Lang::Yaml => "yaml",
        Lang::Xml => "xml",
        Lang::Sql => "sql",
        Lang::Markdown => "markdown",
        Lang::Shell => "bash",
        Lang::Plain => {
            let lower = path.to_ascii_lowercase();
            if [".html", ".htm", ".svg"].iter().any(|e| lower.ends_with(e)) {
                "html"
            } else {
                ""
            }
        }
    }
}

pub fn system_prompt(rules: &[Rule], output_format: &str) -> String {
    let rules_part = if rules.is_empty() {
        String::new()
    } else {
        let parts: Vec<String> = rules
            .iter()
            .map(|r| format!("## {}\n\n{}", r.path, super::js::trim(&r.content)))
            .collect();
        format!("# Project rules\n\n{}", parts.join("\n\n"))
    };
    [
        rules_part.as_str(),
        super::js::trim(output_format),
        FILE_FORMAT,
    ]
    .into_iter()
    .filter(|p| !p.is_empty())
    .collect::<Vec<_>>()
    .join("\n\n")
}

/// The files block prepended to a user message when the full context is (re)sent.
pub fn context_block(files: &[ContextFile]) -> String {
    let mut lines = vec![format!("# Context files ({})", files.len()), String::new()];
    for f in files {
        let Some(content) = &f.content else { continue };
        let header = if f.condensed {
            format!("### FILE: {} {REFERENCE_NOTE}", f.path)
        } else {
            format!("### FILE: {}", f.path)
        };
        let note = if f.condensed {
            "Embedded images, fonts, minified scripts/styles and long path data were replaced by placeholders.\n"
        } else {
            ""
        };
        lines.push(format!(
            "{header}\n{note}```{}\n{content}\n```",
            fence_lang(&f.path)
        ));
    }
    let images: Vec<&ContextFile> = files.iter().filter(|f| f.image.is_some()).collect();
    if !images.is_empty() {
        lines.push(String::new());
        lines.push("Image files (attached as pictures below):".into());
        lines.extend(images.iter().map(|f| format!("- {}", f.path)));
    }
    let missing: Vec<&ContextFile> = files
        .iter()
        .filter(|f| f.content.is_none() && f.image.is_none())
        .collect();
    if !missing.is_empty() {
        lines.push(String::new());
        lines.push("Also selected, contents not available:".into());
        lines.extend(missing.iter().map(|f| format!("- {}", f.path)));
    }
    lines.join("\n")
}

/// Text plus pictures, each picture introduced by its name so the model can refer to it.
pub fn with_images(text: String, images: &[ContextImage]) -> ApiContent {
    if images.is_empty() {
        return ApiContent::Text(text);
    }
    let mut parts = vec![ContentPart::Text { text }];
    for img in images {
        parts.push(ContentPart::Text {
            text: format!("Image: {}", img.name),
        });
        parts.push(ContentPart::ImageUrl {
            image_url: ImageUrl {
                url: img.url.clone(),
                detail: Some(ImageDetail::High),
            },
        });
    }
    ApiContent::Parts(parts)
}

/// Inputs for [`build_turn`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnInput {
    pub text: String,
    /// New or changed files (attached in full).
    pub files: Vec<ContextFile>,
    /// Still selected and unchanged since sent earlier in this conversation.
    pub unchanged: Vec<String>,
    /// Sent earlier but no longer selected.
    pub removed: Vec<String>,
    pub attachments: Vec<ContextImage>,
}

fn bullets(paths: &[String]) -> String {
    paths
        .iter()
        .map(|p| format!("- {p}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The new user turn. Only files that are new to this conversation or changed since they were
/// last sent are attached; the model already has the others from earlier turns.
pub fn build_turn(turn: &TurnInput) -> ApiContent {
    let mut sections = Vec::new();
    if !turn.files.is_empty() {
        sections.push(context_block(&turn.files));
    }
    if !turn.unchanged.is_empty() {
        sections.push(format!(
            "# Unchanged files\n\nStill in context, unchanged since sent earlier in this conversation:\n{}",
            bullets(&turn.unchanged)
        ));
    }
    if !turn.removed.is_empty() {
        sections.push(format!(
            "# Removed from context\n\nIgnore these files from earlier messages:\n{}",
            bullets(&turn.removed)
        ));
    }
    let body = if sections.is_empty() {
        turn.text.clone()
    } else {
        format!("{}\n\n# Request\n\n{}", sections.join("\n\n"), turn.text)
    };
    let images: Vec<ContextImage> = turn
        .files
        .iter()
        .filter_map(|f| {
            f.image.as_ref().map(|url| ContextImage {
                name: f.path.clone(),
                url: url.clone(),
            })
        })
        .chain(turn.attachments.iter().cloned())
        .collect();
    with_images(body, &images)
}

/// Plain text of a turn, for token estimates.
pub fn content_text(content: &ApiContent) -> String {
    match content {
        ApiContent::Text(t) => t.clone(),
        ApiContent::Parts(parts) => parts
            .iter()
            .map(|p| match p {
                ContentPart::Text { text } => text.as_str(),
                ContentPart::ImageUrl { .. } => "",
            })
            .collect::<Vec<_>>()
            .join("\n"),
    }
}

/// Pictures in a turn.
pub fn content_images(content: &ApiContent) -> usize {
    match content {
        ApiContent::Text(_) => 0,
        ApiContent::Parts(parts) => parts
            .iter()
            .filter(|p| matches!(p, ContentPart::ImageUrl { .. }))
            .count(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_serialises_like_ts() {
        let c = with_images(
            "hi".into(),
            &[ContextImage {
                name: "a.png".into(),
                url: "data:x".into(),
            }],
        );
        assert_eq!(
            serde_json::to_string(&c).unwrap(),
            r#"[{"type":"text","text":"hi"},{"type":"text","text":"Image: a.png"},{"type":"image_url","image_url":{"url":"data:x","detail":"high"}}]"#
        );
        assert_eq!(
            serde_json::to_string(&ApiContent::Text("x".into())).unwrap(),
            r#""x""#
        );
        assert_eq!(content_images(&c), 1);
        assert_eq!(content_text(&c), "hi\nImage: a.png\n");
    }

    #[test]
    fn system_prompt_without_rules() {
        assert_eq!(system_prompt(&[], " "), FILE_FORMAT);
    }
}
