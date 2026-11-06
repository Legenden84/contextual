//! Short Foundry status text for the status bar and welcome screen (`lib/foundry-status.ts`).

use serde::{Deserialize, Serialize};

/// The parts of a configured health report the label needs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadyHealth {
    pub endpoint: String,
    pub deployment: String,
}

/// Connection state of the model endpoint, as tracked by the app.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "lowercase")]
pub enum FoundryStatus {
    Checking,
    Ready {
        health: ReadyHealth,
    },
    Unconfigured {
        missing: Vec<String>,
        invalid: Vec<String>,
    },
    Offline {
        error: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Tone {
    Ok,
    Warn,
    Muted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FoundryLabel {
    pub text: String,
    pub tone: Tone,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

pub fn describe_foundry(status: &FoundryStatus) -> FoundryLabel {
    let label = |text: String, tone, detail| FoundryLabel { text, tone, detail };
    match status {
        FoundryStatus::Checking => label("Foundry: checking…".into(), Tone::Muted, None),
        FoundryStatus::Ready { health } => label(
            format!("Foundry: {}", health.deployment),
            Tone::Ok,
            Some(health.endpoint.clone()),
        ),
        FoundryStatus::Unconfigured { missing, invalid } => {
            let all: Vec<&str> = missing.iter().chain(invalid).map(String::as_str).collect();
            label(
                "Foundry: not configured".into(),
                Tone::Warn,
                Some(format!(
                    "Set {} in .env.local and restart npm run dev.",
                    all.join(", ")
                )),
            )
        }
        FoundryStatus::Offline { error } => label(
            "Foundry: local server unavailable".into(),
            Tone::Warn,
            Some(error.clone()),
        ),
    }
}
