//! Entry point: JSON-RPC on stdin/stdout, logs on stderr (`RUST_LOG` to adjust).

use std::sync::Arc;

use contextual_core::host::LocalHost;
use tokio::io::BufReader;

#[tokio::main]
async fn main() -> std::io::Result<()> {
    if std::env::args().any(|a| a == "--version") {
        println!("contextual-agent {}", contextual_agent::AGENT_VERSION);
        return Ok(());
    }
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    tracing::info!("contextual-agent {} started", contextual_agent::AGENT_VERSION);
    let stdin = BufReader::new(tokio::io::stdin());
    contextual_agent::serve(stdin, tokio::io::stdout(), Arc::new(LocalHost::new())).await
}
