//! Host-side command preparation without subprocess execution.
//!
//! Executable lookup never invokes a login shell, uses PATHEXT, translates paths between
//! operating systems, or launches a platform-specific script wrapper implicitly.
//! The existing Git runner and its environment policy are unaffected.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use contextual_protocol::commands::{CommandPlatform, PrepareCommand, PreparedCommand};

use super::{Command, CommandRequest, Interpreter, WorkingDirectory, parse_arguments};
use crate::{Error, Result};

/// Versioned independently from model arguments so environment-policy changes invalidate
/// previously prepared snapshots.
pub const ENVIRONMENT_POLICY: &str = "minimal-v1";

pub(super) const MAX_ENVIRONMENT_BYTES: usize = 32 * 1024;

/// Parent variables that may enter the explicitly constructed command environment.
/// No credentials, agent sockets, loader variables or tool-specific option variables.
pub(super) const BASE_VARIABLES: &[&str] = &[
    "PATH",
    "HOME",
    "USER",
    "LOGNAME",
    "USERPROFILE",
    "SystemRoot",
    "WINDIR",
    "TEMP",
    "TMP",
    "TMPDIR",
    "LANG",
    "LC_ALL",
];

/// Stable for the lifetime of one LocalHost, including its clones.
/// A new host/agent receives a new identity even when it serves the same project.
pub struct CommandPreparer {
    epoch: String,
}

impl Default for CommandPreparer {
    fn default() -> Self {
        Self {
            epoch: uuid::Uuid::new_v4().to_string(),
        }
    }
}

impl CommandPreparer {
    /// Performs filesystem and environment inspection only.
    ///
    /// `allowed_root` must be the canonical root freshly checked by the host's native
    /// command allowlist check. This method belongs on the blocking pool.
    pub fn prepare(&self, allowed_root: &Path, request: PrepareCommand) -> Result<PreparedCommand> {
        let command = parse_arguments(&request.command_json)?;
        let environment = restricted_environment(std::env::vars_os(), &command)?;
        self.build(allowed_root, request, command, environment)
    }

    fn build(
        &self,
        allowed_root: &Path,
        request: PrepareCommand,
        command: CommandRequest,
        environment: BTreeMap<String, String>,
    ) -> Result<PreparedCommand> {
        if request.project_epoch.is_empty()
            || request.project_epoch.len() > 256
            || request.project_epoch.chars().any(char::is_control)
        {
            return Err(Error::invalid(
                "A bounded project-instance identity is required.",
            ));
        }
        let directory = WorkingDirectory::resolve(Path::new(&request.root), &command.cwd)?;
        if directory.root() != allowed_root {
            return Err(Error::conflict(
                "Project directory changed during command preparation.",
            ));
        }

        let platform = platform()?;
        let (program, args) = invocation(&command.command, platform)?;
        let program = resolve_program(
            &program,
            directory.cwd(),
            environment.get("PATH").map(String::as_str),
        )?;

        Ok(PreparedCommand {
            root: wire_path(directory.root())?,
            cwd: wire_path(directory.cwd())?,
            program: wire_path(&program)?,
            args,
            environment,
            environment_policy: ENVIRONMENT_POLICY.into(),
            host_epoch: self.epoch.clone(),
            platform,
            request,
        })
    }
}

fn platform() -> Result<CommandPlatform> {
    if cfg!(windows) {
        Ok(CommandPlatform::Windows)
    } else if cfg!(target_os = "macos") {
        Ok(CommandPlatform::Macos)
    } else if cfg!(target_os = "linux") {
        Ok(CommandPlatform::Linux)
    } else {
        Err(Error::tool(
            "Command preparation is unsupported on this platform.",
        ))
    }
}

fn wire_path(path: &Path) -> Result<String> {
    path.to_str()
        .map(str::to_owned)
        .ok_or_else(|| Error::invalid("Command paths must be representable as UTF-8."))
}

fn invocation(command: &Command, platform: CommandPlatform) -> Result<(String, Vec<String>)> {
    let Command::Shell {
        interpreter,
        script,
    } = command
    else {
        let Command::Exec { program, args } = command else {
            return Err(Error::internal("Unknown command representation."));
        };
        return Ok((program.clone(), args.clone()));
    };

    let windows = platform == CommandPlatform::Windows;
    let (program, flags): (&str, &[&str]) = match interpreter {
        Interpreter::Sh if !windows => ("sh", &["-c"]),
        Interpreter::Bash if !windows => ("bash", &["--noprofile", "--norc", "-c"]),
        Interpreter::Zsh if !windows => ("zsh", &["-f", "-c"]),
        Interpreter::Pwsh => (
            if windows { "pwsh.exe" } else { "pwsh" },
            &["-NoLogo", "-NoProfile", "-NonInteractive", "-Command"],
        ),
        Interpreter::WindowsPowerShell if windows => (
            "powershell.exe",
            &["-NoLogo", "-NoProfile", "-NonInteractive", "-Command"],
        ),
        Interpreter::Cmd => {
            return Err(Error::tool(
                "cmd scripts are not supported yet. Exact Windows shell command-line \
                 handling must be implemented before they can be offered for execution.",
            ));
        }
        _ => {
            return Err(Error::tool(
                "The requested shell interpreter is unsupported on this host. \
                 No substitute interpreter was selected.",
            ));
        }
    };
    let mut args: Vec<String> = flags.iter().map(|flag| (*flag).to_owned()).collect();
    args.push(script.clone());
    Ok((program.to_owned(), args))
}

fn restricted_environment(
    parent: impl IntoIterator<Item = (OsString, OsString)>,
    request: &CommandRequest,
) -> Result<BTreeMap<String, String>> {
    request.validate()?;
    let mut environment = BTreeMap::new();
    for (name, value) in parent {
        let Some(name) = name.to_str() else {
            continue;
        };
        let Some(canonical) = BASE_VARIABLES.iter().find(|candidate| {
            if cfg!(windows) {
                candidate.eq_ignore_ascii_case(name)
            } else {
                **candidate == name
            }
        }) else {
            continue;
        };
        let value = value
            .into_string()
            .map_err(|_| Error::invalid("Command environment must be representable as UTF-8."))?;
        if value.contains('\0') || value.len() > MAX_ENVIRONMENT_BYTES {
            return Err(Error::invalid(
                "Command environment value is invalid or too large.",
            ));
        }
        if environment.insert((*canonical).to_owned(), value).is_some() {
            return Err(Error::invalid(
                "Command environment contains duplicate variable names.",
            ));
        }
    }

    // Empty/relative PATH entries can search the app's cwd or change meaning when the
    // command cwd changes. Exclude them and show the resulting exact PATH in preparation.
    let search_path = clean_search_path(&environment)?;
    environment.insert("PATH".into(), search_path);
    for entry in &request.environment {
        environment.insert(entry.name.clone(), entry.value.clone());
    }

    let bytes: usize = environment
        .iter()
        .map(|(name, value)| name.len() + value.len())
        .sum();
    if bytes > MAX_ENVIRONMENT_BYTES {
        return Err(Error::invalid(
            "Command environment exceeds the size limit.",
        ));
    }
    Ok(environment)
}

fn clean_search_path(environment: &BTreeMap<String, String>) -> Result<String> {
    let mut directories: Vec<PathBuf> = environment
        .get("PATH")
        .map(|path| {
            std::env::split_paths(path)
                .filter(|p| p.is_absolute())
                .collect()
        })
        .unwrap_or_default();

    // Only use conventional fallback directories if no absolute PATH entries survived.
    // Discovering the user's login-shell PATH would itself execute unapproved code.
    if directories.is_empty() {
        if cfg!(windows) {
            if let Some(root) = environment.get("SystemRoot") {
                let root = PathBuf::from(root);
                if root.is_absolute() {
                    directories.push(root.join("System32"));
                    directories.push(root);
                }
            }
        } else {
            if cfg!(target_os = "macos") {
                directories.push(PathBuf::from("/opt/homebrew/bin"));
            }
            directories.extend(
                ["/usr/local/bin", "/usr/bin", "/bin"]
                    .into_iter()
                    .map(PathBuf::from),
            );
        }
    }
    std::env::join_paths(directories)
        .map_err(|_| Error::invalid("Command PATH is invalid."))?
        .into_string()
        .map_err(|_| Error::invalid("Command PATH must be representable as UTF-8."))
}

fn resolve_program(program: &str, cwd: &Path, search_path: Option<&str>) -> Result<PathBuf> {
    let requested = Path::new(program);
    let mut names = vec![requested.to_path_buf()];
    #[cfg(windows)]
    {
        match requested
            .extension()
            .and_then(|extension| extension.to_str())
        {
            Some(extension)
                if extension.eq_ignore_ascii_case("exe")
                    || extension.eq_ignore_ascii_case("com") => {}
            None => {
                names = vec![
                    requested.with_extension("exe"),
                    requested.with_extension("com"),
                ];
            }
            _ => {
                return Err(Error::tool(
                    "Direct Windows commands must resolve to .exe or .com files. \
                     Script launchers require an explicit supported interpreter.",
                ));
            }
        }
    }
    #[cfg(not(windows))]
    let _ = &mut names;

    let has_directory = requested.is_absolute() || requested.components().count() > 1;
    if has_directory {
        for name in names {
            let candidate = if name.is_absolute() {
                name
            } else {
                cwd.join(name)
            };
            if executable_file(&candidate) {
                return Ok(dunce::canonicalize(candidate)?);
            }
        }
    } else if let Some(search_path) = search_path {
        for directory in std::env::split_paths(search_path).filter(|p| p.is_absolute()) {
            for name in &names {
                let candidate = directory.join(name);
                if executable_file(&candidate) {
                    return Ok(dunce::canonicalize(candidate)?);
                }
            }
        }
    }
    Err(Error::tool(
        "The requested executable was not found as an executable file on this host. \
         No alternative program or shell was selected.",
    ))
}

fn executable_file(path: &Path) -> bool {
    let Ok(metadata) = std::fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::EnvironmentOverride;

    fn request(program: String) -> CommandRequest {
        CommandRequest {
            version: 1,
            purpose: "Inspect command preparation without running the program.".into(),
            command: Command::Exec {
                program,
                args: vec!["a b".into(), "$HOME".into(), "&".into()],
            },
            cwd: ".".into(),
            timeout_seconds: 30,
            environment: Vec::new(),
        }
    }

    fn executable_fixture(root: &Path) -> PathBuf {
        let name = if cfg!(windows) { "tool.exe" } else { "tool" };
        let path = root.join(name);
        std::fs::write(&path, "This preparation fixture must never be executed.\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        path
    }

    #[test]
    fn preparation_preserves_arguments_and_does_not_execute_the_fixture() {
        let tmp = tempfile::tempdir().unwrap();
        let program = executable_fixture(tmp.path());
        let root = dunce::canonicalize(tmp.path()).unwrap();
        let command = request(program.to_str().unwrap().into());
        let expected_args = match &command.command {
            Command::Exec { args, .. } => args.clone(),
            _ => unreachable!(),
        };
        let wire_request = PrepareCommand {
            root: root.to_str().unwrap().into(),
            project_epoch: "project-1".into(),
            command_json: serde_json::to_string(&command).unwrap(),
        };
        let environment = restricted_environment(Vec::new(), &command).unwrap();
        let preparer = CommandPreparer::default();
        let prepared = preparer
            .build(&root, wire_request, command, environment)
            .unwrap();

        assert_eq!(
            prepared.program,
            wire_path(&dunce::canonicalize(program).unwrap()).unwrap()
        );
        assert_eq!(prepared.args, expected_args);
        assert_eq!(prepared.cwd, wire_path(&root).unwrap());
        assert_eq!(prepared.environment_policy, ENVIRONMENT_POLICY);
    }

    #[test]
    fn environment_excludes_secrets_agents_loaders_and_tool_options() {
        let command = request("tool".into());
        let parent = [
            ("AZURE_OPENAI_API_KEY", "secret"),
            ("OPENAI_API_KEY", "secret"),
            ("GH_TOKEN", "secret"),
            ("SSH_AUTH_SOCK", "socket"),
            ("GIT_DIR", "other-repository"),
            ("LD_PRELOAD", "library"),
            ("DYLD_INSERT_LIBRARIES", "library"),
            ("NODE_OPTIONS", "--require injected.js"),
            ("BASH_ENV", "injected.sh"),
            ("ENV", "injected.sh"),
            ("HOME", "/home/example"),
        ];
        let environment = restricted_environment(
            parent
                .into_iter()
                .map(|(name, value)| (name.into(), value.into())),
            &command,
        )
        .unwrap();

        assert_eq!(
            environment.get("HOME").map(String::as_str),
            Some("/home/example")
        );
        assert_eq!(
            environment.len(),
            2,
            "only HOME and constructed PATH should remain"
        );
    }

    #[test]
    fn permitted_overrides_are_exact_and_parent_ci_is_not_inherited() {
        let mut command = request("tool".into());
        command.environment = vec![EnvironmentOverride {
            name: "CI".into(),
            value: "true".into(),
        }];
        let environment = restricted_environment(
            [(OsString::from("CI"), OsString::from("secret-parent-value"))],
            &command,
        )
        .unwrap();
        assert_eq!(environment.get("CI").map(String::as_str), Some("true"));
    }

    #[test]
    fn relative_path_entries_are_excluded() {
        let tmp = tempfile::tempdir().unwrap();
        let path =
            std::env::join_paths([Path::new("."), tmp.path(), Path::new("relative")]).unwrap();
        let command = request("tool".into());
        let environment =
            restricted_environment([(OsString::from("PATH"), path)], &command).unwrap();
        let directories: Vec<PathBuf> = std::env::split_paths(&environment["PATH"]).collect();
        assert_eq!(directories, vec![tmp.path().to_path_buf()]);
    }

    #[test]
    fn bare_programs_resolve_on_the_prepared_path_not_the_app_cwd() {
        let tmp = tempfile::tempdir().unwrap();
        let program = executable_fixture(tmp.path());
        let path = std::env::join_paths([tmp.path()]).unwrap();
        let resolved = resolve_program(
            program.file_name().unwrap().to_str().unwrap(),
            Path::new("unused"),
            path.to_str(),
        )
        .unwrap();
        assert_eq!(resolved, dunce::canonicalize(program).unwrap());
    }

    #[test]
    fn an_explicit_missing_program_does_not_fall_back_to_path() {
        let tmp = tempfile::tempdir().unwrap();
        let program = executable_fixture(tmp.path());
        let path = std::env::join_paths([tmp.path()]).unwrap();
        let missing = tmp
            .path()
            .join("missing")
            .join(program.file_name().unwrap());
        assert!(resolve_program(missing.to_str().unwrap(), tmp.path(), path.to_str()).is_err());
    }

    #[test]
    fn shell_preparation_keeps_the_entire_script_as_one_argument() {
        let script = "printf '%s\\n' \"$HOME\" | head -1 > result.txt\n";
        let (program, args) = invocation(
            &Command::Shell {
                interpreter: Interpreter::Bash,
                script: script.into(),
            },
            CommandPlatform::Linux,
        )
        .unwrap();
        assert_eq!(program, "bash");
        assert_eq!(args, ["--noprofile", "--norc", "-c", script]);
    }

    #[test]
    fn unsupported_interpreters_are_not_substituted() {
        assert!(
            invocation(
                &Command::Shell {
                    interpreter: Interpreter::WindowsPowerShell,
                    script: "Get-Location".into(),
                },
                CommandPlatform::Linux,
            )
            .is_err()
        );
        assert!(
            invocation(
                &Command::Shell {
                    interpreter: Interpreter::Cmd,
                    script: "echo test".into(),
                },
                CommandPlatform::Windows,
            )
            .is_err()
        );
    }

    #[test]
    fn fresh_host_instances_have_different_epochs() {
        let first = CommandPreparer::default();
        let second = CommandPreparer::default();
        assert_ne!(first.epoch, second.epoch);
    }

    #[cfg(windows)]
    #[test]
    fn batch_files_cannot_be_prepared_as_direct_executables() {
        let tmp = tempfile::tempdir().unwrap();
        let program = tmp.path().join("tool.cmd");
        std::fs::write(&program, "echo must not run").unwrap();
        assert!(resolve_program(program.to_str().unwrap(), tmp.path(), None).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn non_executable_files_are_refused() {
        use std::os::unix::fs::PermissionsExt as _;
        let tmp = tempfile::tempdir().unwrap();
        let program = tmp.path().join("tool");
        std::fs::write(&program, "not executable").unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert!(resolve_program(program.to_str().unwrap(), tmp.path(), None).is_err());
    }
}
