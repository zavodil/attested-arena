//! What the judge signs, and what the arena contract verifies.
//!
//! The judge's key is `ed25519`, bound to the judge's build (`bind: "wasm"`)
//! and to no caller: one public key per wasm sha256, derived inside OutLayer's
//! keystore. The arena pins it at initialisation, so a result signed by any
//! other build — including a later build of this same code — is refused.
//!
//! A signature covers [`PREFIX`] followed by the payload's JSON text, byte for
//! byte. The payload names the arena and the operator (from the host, never
//! the input), so a result cannot be replayed into another arena. Bulky
//! evidence travels next to the payload; the payload carries its sha256.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const PREFIX: &[u8] = b"attested-arena/1\n";

/// The bytes the judge's key signs.
pub fn signing_message(payload_json: &str) -> Vec<u8> {
    let mut m = PREFIX.to_vec();
    m.extend_from_slice(payload_json.as_bytes());
    m
}

/// What a judge run prints: the payload as JSON text, its signature, and the
/// evidence as JSON text.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct JudgeOutput {
    pub payload: String,
    /// ed25519, hex.
    pub signature: String,
    pub evidence: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Payload {
    Identity(Identity),
    Batch(Batch),
    Portion(Portion),
    Reveal(Reveal),
}

/// The judge's key, said by the judge.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Identity {
    /// ed25519, hex.
    pub judge_key: String,
}

/// A batch of generated tasks: the leaves of the admitted ones, and why the
/// others were kept out.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Batch {
    pub arena: String,
    pub operator: String,
    pub batch: u32,
    pub family: String,
    /// Hex, in the order the ciphertexts are given.
    pub leaves: Vec<String>,
    /// sha256 of each ciphertext, hex, in the same order.
    pub ciphertexts: Vec<String>,
    /// How many tasks the generator wrote.
    pub generated: u32,
    /// Rejections by reason.
    pub rejected: BTreeMap<String, u32>,
    pub evidence_sha256: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    /// Decides whether the challenger gets a confirmation.
    Test,
    /// Fresh tasks; decides the record, and its gain is measured here.
    Confirm,
}

/// One portion of a duel: both models on the same tasks.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Portion {
    pub arena: String,
    pub operator: String,
    pub duel: u32,
    pub portion: u32,
    pub phase: Phase,
    pub champion: String,
    pub challenger: String,
    /// The tasks, by leaf, in pool order.
    pub leaves: Vec<String>,
    pub results: Vec<Outcome>,
    pub evidence_sha256: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Outcome {
    pub champion: bool,
    pub challenger: bool,
}

/// Tasks no duel used, opened after the season closed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reveal {
    pub arena: String,
    pub operator: String,
    pub leaves: Vec<String>,
    pub evidence_sha256: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_payload_names_its_kind() {
        let p = Payload::Identity(Identity { judge_key: "ab".into() });
        assert_eq!(serde_json::to_string(&p).unwrap(), r#"{"kind":"identity","judge_key":"ab"}"#);
        assert_eq!(signing_message("{}"), b"attested-arena/1\n{}".to_vec());
    }
}
