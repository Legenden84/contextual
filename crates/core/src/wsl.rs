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
        let units: Vec<u16> = body
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
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
        .map(|(i, name)| Distro {
            name: name.to_string(),
            is_default: i == 0,
        })
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
        let lxss = RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey(r"Software\Microsoft\Windows\CurrentVersion\Lxss")?;
        let default: String = lxss.get_value("DefaultDistribution").unwrap_or_default();
        let mut out = Vec::new();
        for guid in lxss.enum_keys().flatten() {
            let Ok(key) = lxss.open_subkey(&guid) else {
                continue;
            };
            let Ok(name) = key.get_value::<String, _>("DistributionName") else {
                continue;
            };
            out.push(Distro {
                name,
                is_default: guid.eq_ignore_ascii_case(&default),
            });
        }
        out.sort_by_key(|d| !d.is_default);
        Ok(out)
    }
}

// ---------------------------------------------------------------- agent deployment (plan §6.6)

/// Where an agent version lives inside the distro.
pub fn agent_dir(version: &str) -> String {
    format!("$HOME/.cache/contextual/agent/{version}")
}

/// Version folder name: the app version plus a hash of the agent binary, so every new build is
/// redeployed and old ones are pruned.
pub fn agent_version(app_version: &str, binary: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let hash = hex::encode(Sha256::digest(binary));
    format!("{app_version}-{}", &hash[..12])
}

/// Prints the CPU architecture, then `installed` when this version is already there.
pub fn probe_script(version: &str) -> String {
    let dir = agent_dir(version);
    format!("uname -m; test -x \"{dir}/contextual-agent\" && echo installed; true")
}

/// Writes stdin to the agent path (atomically), makes it executable and prunes other versions.
pub fn install_script(version: &str) -> String {
    let dir = agent_dir(version);
    format!(
        "set -e; mkdir -p \"{dir}\"; cat > \"{dir}/contextual-agent.tmp\"; chmod +x \"{dir}/contextual-agent.tmp\"; \
         mv \"{dir}/contextual-agent.tmp\" \"{dir}/contextual-agent\"; \
         for d in \"$HOME/.cache/contextual/agent\"/*; do [ \"$d\" = \"{dir}\" ] || rm -rf \"$d\"; done"
    )
}

/// Runs the agent through a login shell, so it sees the user's PATH (git, gh) — `wsl.exe -e`
/// alone starts a bare, non-interactive environment.
pub fn launch_script(version: &str) -> String {
    format!("exec \"{}/contextual-agent\"", agent_dir(version))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Probe {
    pub arch: String,
    pub installed: bool,
}

pub fn parse_probe(output: &str) -> Probe {
    let mut lines = output.lines().map(str::trim).filter(|l| !l.is_empty());
    let arch = lines.next().unwrap_or("").to_string();
    Probe {
        installed: lines.any(|l| l == "installed"),
        arch,
    }
}

/// Starts `contextual-agent` inside a WSL distro, deploying it first when needed.
pub struct WslLauncher {
    pub distro: String,
    pub app_version: String,
    /// `wsl.exe` (configurable for tests).
    pub wsl: std::path::PathBuf,
    /// The bundled Linux agents, by architecture (`x86_64`, `aarch64`).
    pub binaries: Vec<(String, std::path::PathBuf)>,
    /// Extra environment for `wsl.exe` (tests).
    pub env: Vec<(String, String)>,
}

impl WslLauncher {
    fn command(&self, script: &str, login: bool) -> tokio::process::Command {
        let mut cmd = tokio::process::Command::new(&self.wsl);
        cmd.args([
            "-d",
            &self.distro,
            "-e",
            "sh",
            if login { "-lc" } else { "-c" },
            script,
        ]);
        cmd.envs(self.env.iter().map(|(k, v)| (k, v)));
        #[cfg(windows)]
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        cmd
    }

    async fn run(&self, script: &str, stdin: Option<&[u8]>) -> crate::Result<String> {
        use tokio::io::AsyncWriteExt;
        let mut cmd = self.command(script, false);
        cmd.stdin(if stdin.is_some() {
            std::process::Stdio::piped()
        } else {
            std::process::Stdio::null()
        })
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
        let mut child = cmd
            .spawn()
            .map_err(|e| crate::Error::tool(format!("Couldn't run wsl.exe: {e}")))?;
        if let (Some(data), Some(mut pipe)) = (stdin, child.stdin.take()) {
            pipe.write_all(data).await?;
            drop(pipe);
        }
        let out = child.wait_with_output().await?;
        if !out.status.success() {
            let err = decode_wsl_output(&out.stderr);
            let msg = if err.trim().is_empty() {
                decode_wsl_output(&out.stdout)
            } else {
                err
            };
            return Err(crate::Error::tool(format!(
                "WSL ({}): {}",
                self.distro,
                msg.trim()
            )));
        }
        Ok(decode_wsl_output(&out.stdout))
    }
}

#[async_trait::async_trait]
impl crate::remote::AgentLauncher for WslLauncher {
    async fn launch(&self) -> crate::Result<crate::remote::RemoteHost> {
        // Which architecture, and is a matching agent there already?
        let first = self
            .binaries
            .first()
            .ok_or_else(|| crate::Error::tool("No Linux agent is bundled with this build."))?;
        let probe_version = agent_version(&self.app_version, &std::fs::read(&first.1)?);
        let probe = parse_probe(&self.run(&probe_script(&probe_version), None).await?);
        let (_, path) = self
            .binaries
            .iter()
            .find(|(arch, _)| *arch == probe.arch)
            .ok_or_else(|| {
                crate::Error::tool(format!(
                    "No agent for {} ({}) is bundled with this build.",
                    self.distro, probe.arch
                ))
            })?;
        let binary = std::fs::read(path)?;
        let version = agent_version(&self.app_version, &binary);
        let installed = if version == probe_version {
            probe.installed
        } else {
            parse_probe(&self.run(&probe_script(&version), None).await?).installed
        };
        if !installed {
            tracing::info!("deploying contextual-agent {version} to {}", self.distro);
            self.run(&install_script(&version), Some(&binary)).await?;
        }
        crate::remote::RemoteHost::spawn(
            self.command(&launch_script(&version), true),
            &self.app_version,
            Vec::new(),
        )
        .await
    }

    fn describe(&self) -> String {
        format!("WSL: {}", self.distro)
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
            Distro {
                name: "Ubuntu-24.04".into(),
                is_default: true,
            },
            Distro {
                name: "docker-desktop".into(),
                is_default: false,
            },
        ];
        let text = "Ubuntu-24.04\r\ndocker-desktop\r\n\r\n";
        assert_eq!(parse_list_quiet(&utf16(text, false)), expected);
        assert_eq!(parse_list_quiet(text.as_bytes()), expected);
        assert!(parse_list_quiet(b"").is_empty());
    }

    #[test]
    fn deployment_scripts() {
        let v = agent_version("0.1.0", b"binary");
        assert!(v.starts_with("0.1.0-") && v.len() == "0.1.0-".len() + 12);
        assert_ne!(v, agent_version("0.1.0", b"other"));
        assert!(probe_script(&v).contains(&format!("agent/{v}/contextual-agent")));
        assert!(install_script(&v).contains("chmod +x"));
        assert_eq!(
            parse_probe("x86_64\ninstalled\n"),
            Probe {
                arch: "x86_64".into(),
                installed: true
            }
        );
        assert_eq!(
            parse_probe("aarch64\n"),
            Probe {
                arch: "aarch64".into(),
                installed: false
            }
        );
    }
}
