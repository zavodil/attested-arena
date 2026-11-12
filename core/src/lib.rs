//! Attested Arena's rules, as code that the judge runs inside the enclave and
//! that anyone runs again outside it.
//!
//! * [`task`] — what a task is, and the bytes its Merkle leaf is taken over.
//! * [`families`] — the kinds of task, each with a validator, a checker and
//!   the admission rules that decide whether a generated task enters the pool.
//!   A language model writes tasks; it never decides whether an answer is right.
//! * [`request`] and [`answer`] — the exact request a participant model is
//!   sent, and how its answer is read out of the response.
//! * [`merkle`] — the commitment to the pool, published before any duel.
//! * [`sprt`] — the sequential test a duel is decided by.
//! * [`payload`] — the messages the judge signs and the arena verifies.

pub mod answer;
pub mod families;
pub mod merkle;
pub mod payload;
pub mod request;
pub mod sprt;
pub mod task;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// sha256, lowercase hex.
pub fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// A checker's decision on one answer, with the reason when it is refused.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Verdict {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl Verdict {
    pub fn accept() -> Self {
        Verdict { ok: true, reason: None }
    }

    pub fn reject(reason: impl Into<String>) -> Self {
        Verdict { ok: false, reason: Some(reason.into()) }
    }
}

/// A small deterministic generator (SplitMix64) for the choices code makes
/// while building a task. The judge seeds it from the enclave's randomness;
/// the choices it made are recorded in the task, so nobody needs the seed to
/// check anything.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng(seed)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A value in `0..n`; `n` must be positive.
    pub fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n as u64) as usize
    }

    /// `k` distinct indices out of `0..n`, in the order drawn.
    pub fn pick(&mut self, n: usize, k: usize) -> Vec<usize> {
        let mut all: Vec<usize> = (0..n).collect();
        let k = k.min(n);
        for i in 0..k {
            let j = i + self.below(n - i);
            all.swap(i, j);
        }
        all.truncate(k);
        all
    }
}
