//! Entry point: JSON-RPC on stdin/stdout, logs on stderr (`RUST_LOG` to adjust).
//!
//! By default the agent serves files, scans and git, and cannot run anything: that is how
//! the app starts it. Approved command execution is a separate, explicit mode:
//!
//! `contextual-agent --commands --location <local|wsl:NAME> [--journal <absolute dir>]`
//!
//! In that mode the agent also offers the execution service, with this platform's native
//! backend and a durable journal (by default in the agent's data directory). The location is
//! supplied by whoever starts the agent, never by a request. The app passes these arguments
//! only under its debug-only development switch; see
//! `docs/plans/approved-command-execution.md`.

use std::path::PathBuf;
use std::sync::Arc;

use contextual_core::commands::execution::ManagedCommandRunner;
use contextual_core::commands::journal::FileExecutionJournal;
use contextual_core::commands::service::AgentExecutionService;
use contextual_core::host::LocalHost;
use contextual_core::location::Location;
use tokio::io::BufReader;

/// What the command line asked for beyond the default mode.
struct Commands {
    location: Location,
    journal: Option<PathBuf>,
}

fn usage() -> std::io::Error {
    std::io::Error::other(
        "usage: contextual-agent [--version] \
         [--commands --location <local|wsl:NAME> [--journal <absolute dir>]]",
    )
}

/// `Ok(None)`: the default mode. Execution is never enabled by an incomplete or unknown
/// set of arguments: those are an error.
fn parse(arguments: impl Iterator<Item = String>) -> std::io::Result<Option<Commands>> {
    let mut commands = false;
    let mut location = None;
    let mut journal = None;
    let mut arguments = arguments;
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--commands" => commands = true,
            "--location" => {
                let value = arguments.next().ok_or_else(usage)?;
                location = Some(match value.strip_prefix("wsl:") {
                    Some(distro) if !distro.is_empty() => Location::Wsl {
                        distro: distro.to_owned(),
                    },
                    None if value == "local" => Location::Local,
                    _ => return Err(usage()),
                });
            }
            "--journal" => journal = Some(PathBuf::from(arguments.next().ok_or_else(usage)?)),
            _ => return Err(usage()),
        }
    }
    match (commands, location) {
        (false, None) if journal.is_none() => Ok(None),
        (true, Some(location)) => Ok(Some(Commands { location, journal })),
        _ => Err(usage()),
    }
}

fn native_runner() -> Option<Arc<dyn ManagedCommandRunner>> {
    #[cfg(all(target_os = "linux", not(target_env = "uclibc")))]
    return Some(Arc::new(
        contextual_core::commands::execution::linux::LinuxCommandRunner::new(),
    ));
    #[cfg(target_os = "macos")]
    return Some(Arc::new(
        contextual_core::commands::execution::macos::MacosCommandRunner::new(),
    ));
    #[cfg(windows)]
    return Some(Arc::new(
        contextual_core::commands::execution::windows::WindowsCommandRunner::new(),
    ));
    #[allow(unreachable_code)]
    None
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    if std::env::args().any(|a| a == "--version") {
        println!("contextual-agent {}", contextual_agent::AGENT_VERSION);
        return Ok(());
    }
    let commands = parse(std::env::args().skip(1))?;
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    tracing::info!(
        "contextual-agent {} started{}",
        contextual_agent::AGENT_VERSION,
        if commands.is_some() {
            " with command execution"
        } else {
            ""
        }
    );
    let stdin = BufReader::new(tokio::io::stdin());
    let Some(commands) = commands else {
        return contextual_agent::serve(stdin, tokio::io::stdout(), Arc::new(LocalHost::new()))
            .await;
    };

    let runner = native_runner()
        .ok_or_else(|| std::io::Error::other("no native execution backend on this platform"))?;
    let journal = match commands.journal {
        Some(directory) => FileExecutionJournal::open(directory).await,
        None => FileExecutionJournal::open_default().await,
    }
    .map_err(|error| std::io::Error::other(error.message))?;
    let service = Arc::new(AgentExecutionService::new(
        Arc::new(LocalHost::new()),
        commands.location,
        runner,
        Arc::new(journal),
    ));
    contextual_agent::serve_with_execution(stdin, tokio::io::stdout(), service).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(arguments: &[&str]) -> std::io::Result<Option<Commands>> {
        parse(arguments.iter().map(|argument| (*argument).to_owned()))
    }

    #[test]
    fn execution_needs_the_explicit_flag_and_a_location() {
        // The way the app starts it by default: nothing can run.
        assert!(parsed(&[]).unwrap().is_none());

        let wsl = parsed(&["--commands", "--location", "wsl:Ubuntu"])
            .unwrap()
            .unwrap();
        assert_eq!(
            wsl.location,
            Location::Wsl {
                distro: "Ubuntu".into()
            }
        );
        assert!(wsl.journal.is_none());
        let local = parsed(&["--location", "local", "--journal", "/tmp/j", "--commands"])
            .unwrap()
            .unwrap();
        assert_eq!(local.location, Location::Local);
        assert_eq!(local.journal, Some(PathBuf::from("/tmp/j")));

        // Anything incomplete or unknown is an error, never a quiet fallback either way.
        for arguments in [
            &["--commands"][..],
            &["--location", "wsl:Ubuntu"],
            &["--journal", "/tmp/j"],
            &["--commands", "--location"],
            &["--commands", "--location", "wsl:"],
            &["--commands", "--location", "somewhere"],
            &["--commands", "--location", "local", "--run-everything"],
            &["commands"],
        ] {
            assert!(parsed(arguments).is_err(), "{arguments:?}");
        }
    }
}
