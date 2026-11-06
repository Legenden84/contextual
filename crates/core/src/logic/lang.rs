//! Language detection from `src/web/lib/syntax.ts` (`langFromPath`, `langFromFence`), which the
//! response parser and prompt builder need. Highlighting itself is not ported (tree-sitter).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Lang {
    Csharp,
    Typescript,
    Python,
    Json,
    Yaml,
    Xml,
    Sql,
    Markdown,
    Shell,
    Plain,
}

impl Lang {
    /// Display name (`LANG_LABEL`).
    pub fn label(self) -> &'static str {
        match self {
            Self::Csharp => "C#",
            Self::Typescript => "TypeScript",
            Self::Python => "Python",
            Self::Json => "JSON",
            Self::Yaml => "YAML",
            Self::Xml => "XML",
            Self::Sql => "SQL",
            Self::Markdown => "Markdown",
            Self::Shell => "Shell",
            Self::Plain => "Text",
        }
    }
}

/// Language of a file from its extension; `plain` when unknown or no path.
pub fn lang_from_path(path: Option<&str>) -> Lang {
    let Some(path) = path.filter(|p| !p.is_empty()) else {
        return Lang::Plain;
    };
    if path.ends_with(".gitignore") {
        return Lang::Shell;
    }
    let Some((_, ext)) = path.rsplit_once('.') else {
        return Lang::Plain;
    };
    if ext.is_empty() || !ext.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return Lang::Plain;
    }
    match ext.to_ascii_lowercase().as_str() {
        "cs" => Lang::Csharp,
        "ts" | "tsx" | "js" | "jsx" | "mjs" => Lang::Typescript,
        "py" => Lang::Python,
        "json" => Lang::Json,
        "yml" | "yaml" => Lang::Yaml,
        "xml" | "csproj" | "props" | "targets" => Lang::Xml,
        "sql" => Lang::Sql,
        "md" | "mdc" => Lang::Markdown,
        "sh" | "bash" | "ps1" => Lang::Shell,
        _ => Lang::Plain,
    }
}

/// Language of a code fence info string (` ```csharp `).
pub fn lang_from_fence(fence: &str) -> Lang {
    match super::js::trim(fence).to_lowercase().as_str() {
        "cs" | "csharp" | "c#" => Lang::Csharp,
        "ts" | "typescript" | "js" | "javascript" | "tsx" => Lang::Typescript,
        "py" | "python" => Lang::Python,
        "json" => Lang::Json,
        "yaml" | "yml" => Lang::Yaml,
        "xml" | "csproj" => Lang::Xml,
        "sql" => Lang::Sql,
        "md" | "markdown" => Lang::Markdown,
        "sh" | "bash" | "shell" | "powershell" => Lang::Shell,
        _ => Lang::Plain,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_path() {
        assert_eq!(lang_from_path(Some("src/A.CS")), Lang::Csharp);
        assert_eq!(lang_from_path(Some("a/.gitignore")), Lang::Shell);
        assert_eq!(lang_from_path(Some("Makefile")), Lang::Plain);
        assert_eq!(lang_from_path(Some("x.d-ts")), Lang::Plain);
        assert_eq!(lang_from_path(None), Lang::Plain);
    }

    #[test]
    fn from_fence() {
        assert_eq!(lang_from_fence(" C# "), Lang::Csharp);
        assert_eq!(lang_from_fence("rust"), Lang::Plain);
    }
}
