//! Background line diffs shared by chat badges, Changes and the Diff screen.
//! Keys identify immutable published message changes, so cache hits never hash file contents.

use std::cell::Cell;
use std::collections::HashMap;
use std::sync::Arc;

use contextual_core::chat::UiMessage;
use contextual_core::logic::diff::{Diff, DiffOp, compute_diff};
use contextual_core::logic::types::{Change, ChangeStatus};
use gpui_kit::{App, Context, Entity};

use crate::runtime;
use crate::state::chat::ChatModel;

const MAX_MESSAGES: usize = 32;
const MAX_BYTES: usize = 64 * 1024 * 1024;

pub type DiffCache = Entity<DiffStore>;

enum Value {
    Pending,
    Ready {
        diffs: Arc<Vec<Diff>>,
        bytes: usize,
    },
    Failed(String),
}

struct Entry {
    ticket: u64,
    used: Cell<u64>,
    value: Value,
}

#[derive(Default)]
pub struct DiffStore {
    entries: HashMap<String, Entry>,
    clock: Cell<u64>,
    ticket: u64,
}

pub fn message_key(chat: &ChatModel, message: &UiMessage) -> String {
    format!(
        "{}\0{}\0{}",
        chat.thread_generation, chat.thread.id, message.id
    )
}

/// Request only the displayed message (`Some`) or latest proposed changes (`None`).
/// File contents are cloned once on a miss, never for pending or ready entries.
pub fn ensure_message(
    diffs: &DiffCache,
    chat: &Entity<ChatModel>,
    index: Option<usize>,
    cx: &mut App,
) {
    let snapshot = {
        let chat = chat.read(cx);
        let message = match index {
            Some(i) => chat.messages.get(i),
            None => chat.change_message(),
        };
        let Some(message) = message else { return };
        let Some(changes) = message.changes.as_ref().filter(|c| !c.is_empty()) else {
            return;
        };
        let key = message_key(chat, message);
        if diffs.read(cx).contains(&key) {
            return;
        }
        (key, changes.clone())
    };
    diffs.update(cx, |store, cx| store.request(snapshot.0, snapshot.1, cx));
}

fn weight(diffs: &[Diff]) -> usize {
    std::mem::size_of_val(diffs)
        + diffs
            .iter()
            .map(|d| {
                d.ops.capacity() * std::mem::size_of::<DiffOp>()
                    + (d.old_lines.capacity() + d.new_lines.capacity())
                        * std::mem::size_of::<String>()
                    + d.ops.iter().map(|op| op.t.capacity()).sum::<usize>()
                    + d.old_lines
                        .iter()
                        .chain(&d.new_lines)
                        .map(String::capacity)
                        .sum::<usize>()
            })
            .sum::<usize>()
}

impl DiffStore {
    fn touch(&self, key: &str) -> Option<&Entry> {
        let entry = self.entries.get(key)?;
        let now = self.clock.get().wrapping_add(1);
        self.clock.set(now);
        entry.used.set(now);
        Some(entry)
    }

    pub fn contains(&self, key: &str) -> bool {
        self.touch(key).is_some()
    }

    pub fn get(&self, key: &str) -> Option<Arc<Vec<Diff>>> {
        match &self.touch(key)?.value {
            Value::Ready { diffs, .. } => Some(diffs.clone()),
            _ => None,
        }
    }

    pub fn error(&self, key: &str) -> Option<&str> {
        match &self.entries.get(key)?.value {
            Value::Failed(error) => Some(error),
            _ => None,
        }
    }

    fn request(&mut self, key: String, changes: Vec<Change>, cx: &mut Context<Self>) {
        if self.contains(&key) {
            return;
        }
        self.ticket = self.ticket.wrapping_add(1);
        let ticket = self.ticket;
        self.entries.insert(
            key.clone(),
            Entry {
                ticket,
                used: Cell::new(self.clock.get()),
                value: Value::Pending,
            },
        );
        let job = runtime::spawn(cx, async move {
            tokio::task::spawn_blocking(move || {
                let diffs: Vec<_> = changes
                    .iter()
                    .map(|change| {
                        let (old, new) = match change.status {
                            ChangeStatus::A => ("", change.new.as_str()),
                            ChangeStatus::D => (change.old.as_str(), ""),
                            ChangeStatus::M => (change.old.as_str(), change.new.as_str()),
                        };
                        compute_diff(old, new)
                    })
                    .collect();
                let bytes = weight(&diffs);
                (diffs, bytes)
            })
            .await
            .map_err(|e| {
                contextual_core::Error::internal(format!("Could not calculate diff: {e}"))
            })
        });
        cx.spawn(async move |this, cx| {
            let result = job.await;
            let _ = this.update(cx, |this, cx| {
                if this.finish(&key, ticket, result) {
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn finish(
        &mut self,
        key: &str,
        ticket: u64,
        result: contextual_core::Result<(Vec<Diff>, usize)>,
    ) -> bool {
        let Some(entry) = self.entries.get_mut(key) else {
            return false;
        };
        if entry.ticket != ticket || !matches!(&entry.value, Value::Pending) {
            return false;
        }
        entry.value = match result {
            Ok((diffs, bytes)) => Value::Ready {
                diffs: Arc::new(diffs),
                bytes,
            },
            Err(error) => Value::Failed(error.message),
        };
        self.evict(key);
        true
    }

    fn evict(&mut self, keep: &str) {
        loop {
            let bytes: usize = self
                .entries
                .values()
                .map(|entry| match &entry.value {
                    Value::Ready { bytes, .. } => *bytes,
                    _ => 0,
                })
                .sum();
            if self.entries.len() <= MAX_MESSAGES && bytes <= MAX_BYTES {
                break;
            }
            // Never cancel in-flight work. A single oversized result may occupy the cache
            // alone; active views can also retain their own Arc until they leave that reply.
            let victim = self
                .entries
                .iter()
                .filter(|(key, entry)| {
                    key.as_str() != keep && !matches!(&entry.value, Value::Pending)
                })
                .min_by_key(|(_, entry)| entry.used.get())
                .map(|(key, _)| key.clone());
            let Some(victim) = victim else { break };
            self.entries.remove(&victim);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pending(store: &mut DiffStore, key: &str, ticket: u64) {
        store.entries.insert(
            key.into(),
            Entry {
                ticket,
                used: Cell::new(0),
                value: Value::Pending,
            },
        );
    }

    #[test]
    fn result_is_shared_and_stale_completion_is_ignored() {
        let mut store = DiffStore::default();
        pending(&mut store, "reply", 2);
        assert!(!store.finish("reply", 1, Ok((Vec::new(), 0))));
        assert!(store.get("reply").is_none());
        assert!(store.finish("reply", 2, Ok((vec![compute_diff("a", "b")], 0))));
        let first = store.get("reply").unwrap();
        let second = store.get("reply").unwrap();
        assert!(Arc::ptr_eq(&first, &second));
        assert!(!store.finish("reply", 2, Ok((Vec::new(), 0))));
    }

    #[test]
    fn cache_evicts_old_results_by_memory_budget() {
        let mut store = DiffStore::default();
        pending(&mut store, "old", 1);
        assert!(store.finish("old", 1, Ok((Vec::new(), MAX_BYTES))));
        pending(&mut store, "new", 2);
        assert!(store.finish("new", 2, Ok((Vec::new(), 1))));
        assert!(!store.contains("old"));
        assert!(store.get("new").is_some());
    }
}
