//! WSL distro discovery and `wsl.exe` output decoding (plan §6.6). Used by the Windows app.
//!
//! Distros are read from the registry (`HKCU\Software\Microsoft\Windows\CurrentVersion\Lxss`):
//! no process, no encoding issues. `wsl.exe --list --quiet` is the fallback; its output is
//! UTF-16LE by default but UTF-8 with `WSL_UTF8=1` (and on some machines without it), so the
//! encoding is detected per call.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Distro {
    pub name: String,
    pub is_default: bool,
}

/// Decodes `wsl.exe` output: UTF-16LE if it starts with `FF FE` or contains NULs, else UTF-8.
pub fn decode_wsl_output(bytes: &[u8]) -> String {
    let utf16 = bytes.starts_with(&[0xFF, 0xFE]) || bytes.contains(&0);
    let text = if utf16 {
        let body = bytes.strip_prefix(&[0xFF, 0xFE]).unwrap_or(bytes);
        let units: Vec<u16> = body.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        String::from_utf16_lossy(&units)
    } else {
        String::from_utf8_lossy(bytes).into_owned()
    };
    text.trim_start_matches('\u{feff}').to_string()
}

/// Parses `wsl.exe --list --quiet`: one distro per line; the first is the default.
pub fn parse_list_quiet(bytes: &[u8]) -> Vec<Distro> {
    decode_wsl_output(bytes)
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .enumerate()
        .map(|(i, name)| Distro { name: name.to_string(), is_default: i == 0 })
        .collect()
}

/// Installed distros, default first. Empty when WSL isn't installed.
#[cfg(windows)]
pub fn list_distros() -> Vec<Distro> {
    let from_registry = registry::list().unwrap_or_default();
    if !from_registry.is_empty() {
        return from_registry;
    }
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    std::process::Command::new("wsl.exe")
        .args(["--list", "--quiet"])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| parse_list_quiet(&o.stdout))
        .unwrap_or_default()
}

/// WSL exists only on Windows.
#[cfg(not(windows))]
pub fn list_distros() -> Vec<Distro> {
    Vec::new()
}

#[cfg(windows)]
mod registry {
    use super::Distro;
    use winreg::RegKey;
    use winreg::enums::HKEY_CURRENT_USER;

    pub fn list() -> std::io::Result<Vec<Distro>> {
        let lxss = RegKey::predef(HKEY_CURRENT_USER).open_subkey(r"Software\Microsoft\Windows\CurrentVersion\Lxss")?;
        let default: String = lxss.get_value("DefaultDistribution").unwrap_or_default();
        let mut out = Vec::new();
        for guid in lxss.enum_keys().flatten() {
            let Ok(key) = lxss.open_subkey(&guid) else { continue };
            let Ok(name) = key.get_value::<String, _>("DistributionName") else { continue };
            out.push(Distro { name, is_default: guid.eq_ignore_ascii_case(&default) });
        }
        out.sort_by_key(|d| !d.is_default);
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utf16(s: &str, bom: bool) -> Vec<u8> {
        let mut out = if bom { vec![0xFF, 0xFE] } else { vec![] };
        out.extend(s.encode_utf16().flat_map(u16::to_le_bytes));
        out
    }

    #[test]
    fn decodes_both_encodings() {
        let text = "Ubuntu\r\nDebian\r\n";
        assert_eq!(decode_wsl_output(&utf16(text, false)), text);
        assert_eq!(decode_wsl_output(&utf16(text, true)), text);
        assert_eq!(decode_wsl_output(text.as_bytes()), text);
        assert_eq!(decode_wsl_output("\u{feff}Ubuntu".as_bytes()), "Ubuntu");
    }

    #[test]
    fn parses_list() {
        let expected = vec![
            Distro { name: "Ubuntu-24.04".into(), is_default: true },
            Distro { name: "docker-desktop".into(), is_default: false },
        ];
        let text = "Ubuntu-24.04\r\ndocker-desktop\r\n\r\n";
        assert_eq!(parse_list_quiet(&utf16(text, false)), expected);
        assert_eq!(parse_list_quiet(text.as_bytes()), expected);
        assert!(parse_list_quiet(b"").is_empty());
    }
}
