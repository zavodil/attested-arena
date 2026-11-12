//! The kinds of task.
//!
//! Each family has three pieces of code, and a language model has no say in
//! any of them:
//!
//! * a **validator** — the task is well formed;
//! * a **checker** — an answer solves the task;
//! * **admission** — a generated task enters the pool only if it passes
//!   every rule, borrowed from how olympiad problems are prepared (Codeforces
//!   Polygon, the Kattis/ICPC problem package format): the author's own
//!   solution is accepted, known wrong answers are refused, the task's
//!   meaning is confirmed independently, and a task the checker cannot decide
//!   is dropped rather than scored.
//!
//! A model writes the tasks inside the TEE. Code decides whether they are
//! sound and whether an answer is right.

pub mod lines;
pub mod seating;

use serde::{Deserialize, Serialize};

/// Why a generated task was kept out of the pool. The judge counts these per
/// batch and signs the counts, so the admission rate is public.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Rejection {
    /// The generator's output is not a task of this family (schema, sizes,
    /// names, bounds).
    Malformed(String),
    /// The author's own solution is refused by the checker.
    WitnessRefused(String),
    /// No answer satisfies the task.
    Unsolvable,
    /// More answers satisfy the task than the family allows.
    TooManySolutions(usize),
    /// A known wrong answer (a trivial or leaked one) is accepted.
    BaselineAccepted(String),
    /// A damaged copy of the author's solution is accepted: the task does not
    /// tell right from wrong.
    MutationAccepted(String),
    /// A second model, reading only the statement, understood a different task.
    StatementMismatch(String),
}

impl Rejection {
    /// A stable name, for the counts a batch signs.
    pub fn code(&self) -> &'static str {
        match self {
            Rejection::Malformed(_) => "malformed",
            Rejection::WitnessRefused(_) => "witness_refused",
            Rejection::Unsolvable => "unsolvable",
            Rejection::TooManySolutions(_) => "too_many_solutions",
            Rejection::BaselineAccepted(_) => "baseline_accepted",
            Rejection::MutationAccepted(_) => "mutation_accepted",
            Rejection::StatementMismatch(_) => "statement_mismatch",
        }
    }
}

impl std::fmt::Display for Rejection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Rejection::Malformed(why) => write!(f, "malformed: {why}"),
            Rejection::WitnessRefused(why) => write!(f, "the author's solution is refused: {why}"),
            Rejection::Unsolvable => write!(f, "no answer satisfies the task"),
            Rejection::TooManySolutions(n) => write!(f, "{n} or more answers satisfy the task"),
            Rejection::BaselineAccepted(which) => write!(f, "a known wrong answer is accepted: {which}"),
            Rejection::MutationAccepted(which) => write!(f, "a damaged solution is accepted: {which}"),
            Rejection::StatementMismatch(why) => write!(f, "the statement says something else: {why}"),
        }
    }
}

/// The JSON a generator answered with, out of a chat completion's text: the
/// first `{` to the last `}`, so a model that wraps it in prose or a code
/// fence still reads.
pub fn json_object(text: &str) -> Option<&str> {
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    (end > start).then(|| &text[start..=end])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_is_found_inside_prose_and_fences() {
        assert_eq!(json_object("Here:\n```json\n{\"a\": 1}\n```"), Some("{\"a\": 1}"));
        assert_eq!(json_object("no json"), None);
    }
}
