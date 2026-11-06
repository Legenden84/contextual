//! Pull-request helpers ported from `PullRequestDialog.tsx`: the suggested branch name, the
//! default title and the chat notes sent when drafting the description.

use super::prompt::{ApiContent, content_text};

/// "contextual/add-order-cancellation" from the first commit or request.
pub fn suggest_branch(text: &str) -> String {
    let lower = text.to_lowercase();
    // Drop `inline code` spans.
    let mut cleaned = String::new();
    let mut in_code = false;
    for ch in lower.chars() {
        if ch == '`' {
            in_code = !in_code;
            cleaned.push(' ');
        } else if !in_code {
            cleaned.push(ch);
        }
    }
    let slug: Vec<String> = cleaned
        .split(|c: char| !(c.is_ascii_lowercase() || c.is_ascii_digit()))
        .filter(|s| !s.is_empty())
        .take(6)
        .map(str::to_string)
        .collect();
    let slug = slug.join("-");
    format!(
        "contextual/{}",
        if slug.is_empty() { "changes" } else { &slug }
    )
}

/// The title before drafting: the only commit's subject, else the first line of the request
/// (72 chars).
pub fn default_title(commit_subjects: &[String], first_ask: &str) -> String {
    if commit_subjects.len() == 1 {
        return commit_subjects[0].clone();
    }
    let first = commit_subjects
        .last()
        .map(String::as_str)
        .unwrap_or(first_ask);
    first
        .lines()
        .next()
        .unwrap_or("")
        .chars()
        .take(72)
        .collect()
}

/// The conversation as short notes for the PR draft: code replaced by `[code]`, each turn
/// capped at 4,000 chars, the user's turns reduced to their request.
pub fn notes(transcript: &[(bool, &ApiContent)]) -> String {
    transcript
        .iter()
        .map(|(is_user, content)| {
            let text = content_text(content);
            let text = strip_code(&text);
            let text: String = text.chars().take(4000).collect();
            if *is_user {
                let request = text
                    .rsplit("# Request")
                    .next()
                    .unwrap_or(&text)
                    .trim()
                    .to_string();
                format!("Request: {request}")
            } else {
                format!("Reply: {text}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn strip_code(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(start) = rest.find("```") {
        out.push_str(&rest[..start]);
        match rest[start + 3..].find("```") {
            Some(end) => {
                out.push_str("[code]");
                rest = &rest[start + 3 + end + 3..];
            }
            None => {
                rest = &rest[start..];
                break;
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn branch_names() {
        assert_eq!(
            suggest_branch("Add order cancellation!"),
            "contextual/add-order-cancellation"
        );
        assert_eq!(
            suggest_branch("Make `Order.Cancel()` refuse shipped orders and log why"),
            "contextual/make-refuse-shipped-orders-and-log"
        );
        assert_eq!(suggest_branch("???"), "contextual/changes");
    }

    #[test]
    fn titles_and_notes() {
        assert_eq!(default_title(&["Fix it".into()], "ask"), "Fix it");
        assert_eq!(
            default_title(&["b".into(), "a first".into()], "ask"),
            "a first"
        );
        assert_eq!(default_title(&[], "Line one\nline two"), "Line one");
        let user = ApiContent::Text("# Context\nfiles\n\n# Request\nPlease fix".into());
        let reply = ApiContent::Text("Done:\n```rs\nfn a() {}\n```\nok".into());
        assert_eq!(
            notes(&[(true, &user), (false, &reply)]),
            "Request: Please fix\n\nReply: Done:\n[code]\nok"
        );
    }
}
