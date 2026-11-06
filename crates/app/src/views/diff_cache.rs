//! Line diffs of proposed changes, computed once per (path, old, new) and shared by the Changes
//! panel and the Diff screen.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use contextual_core::logic::diff::{Diff, compute_diff};
use contextual_core::logic::hash::hash_text;
use contextual_core::logic::types::{Change, ChangeStatus};

#[derive(Default, Clone)]
pub struct DiffCache(Rc<RefCell<HashMap<String, Rc<Diff>>>>);

impl DiffCache {
    pub fn get(&self, change: &Change) -> Rc<Diff> {
        let key = format!(
            "{}\0{}\0{}",
            change.path,
            hash_text(&change.old),
            hash_text(&change.new)
        );
        if let Some(d) = self.0.borrow().get(&key) {
            return d.clone();
        }
        let (old, new) = match change.status {
            ChangeStatus::D => (change.old.as_str(), ""),
            ChangeStatus::A => ("", change.new.as_str()),
            ChangeStatus::M => (change.old.as_str(), change.new.as_str()),
        };
        let d = Rc::new(compute_diff(old, new));
        let mut map = self.0.borrow_mut();
        if map.len() > 200 {
            map.clear();
        }
        map.insert(key, d.clone());
        d
    }
}
