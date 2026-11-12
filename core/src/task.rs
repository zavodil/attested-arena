//! A task, as it is sealed in the pool and revealed after its duel.
//!
//! The leaf committed on chain is `sha256(0x00 ‖ task_json)`, where
//! `task_json` is [`Task::to_json`]: serde's output for these types, whose
//! field order is fixed by their declarations and whose lists keep their
//! order. The same task always gives the same bytes, so the revealed JSON is
//! checked against the leaf byte for byte. The `salt` makes a leaf
//! unguessable: without it, a small task could be found by hashing candidates.

use crate::families::{lines, seating};
use crate::merkle;
use crate::Verdict;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "family", rename_all = "snake_case", deny_unknown_fields)]
pub enum Task {
    Seating(seating::Seating),
    Lines(lines::Lines),
}

impl Task {
    pub fn to_json(&self) -> Vec<u8> {
        serde_json::to_vec(self).expect("a task always serialises")
    }

    pub fn from_json(bytes: &[u8]) -> Result<Self, String> {
        let task: Task = serde_json::from_slice(bytes).map_err(|e| format!("the task does not read: {e}"))?;
        if task.to_json() != bytes {
            return Err("the task's bytes are not its canonical JSON".to_string());
        }
        Ok(task)
    }

    pub fn leaf(&self) -> [u8; 32] {
        merkle::leaf_hash(&self.to_json())
    }

    pub fn family(&self) -> &'static str {
        match self {
            Task::Seating(_) => "seating",
            Task::Lines(_) => "lines",
        }
    }

    /// What the participant model reads: the statement and the family's
    /// answer format, both fixed by code.
    pub fn prompt(&self) -> String {
        match self {
            Task::Seating(t) => t.prompt(),
            Task::Lines(t) => t.prompt(),
        }
    }

    /// The checker: whether `answer` (the text between `<answer>` tags)
    /// solves the task.
    pub fn check(&self, answer: &str) -> Verdict {
        match self {
            Task::Seating(t) => t.check(answer),
            Task::Lines(t) => t.check(answer),
        }
    }

    /// The validator: whether the task is well formed (sizes, names, bounds).
    pub fn validate(&self) -> Result<(), String> {
        match self {
            Task::Seating(t) => t.validate(),
            Task::Lines(t) => t.validate(),
        }
    }
}
