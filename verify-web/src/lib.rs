//! The report's in-browser checks, as WebAssembly.
//!
//! The page sends one JSON request and gets one JSON answer through
//! [`call`]. The checks are the judge's own code from `arena-core` — the
//! same checkers, leaf hashing, Merkle tree and request builder the judge ran
//! in the enclave — plus the two signature checks: the judge's ed25519 key
//! and a model's EIP-191 response signature.
//!
//! | `op` | input | answer |
//! |---|---|---|
//! | `leaf` | `task` (JSON text) | `{leaf, family}` or `{error}` when the text is not a canonical, valid task |
//! | `root` | `leaves` (hex) | `{root}` |
//! | `grade` | `task`, `response` (the completion's JSON text) | `{ok, reason, answer}` |
//! | `request` | `model`, `task` | `{sha256}` of the exact request the judge sent |
//! | `nonce` | `hashes` | `{nonce}` a model was attested for |
//! | `judge` | `key`, `payload`, `signature` (hex) | `{ok}` — ed25519 over the arena's signing message |
//! | `eip191` | `text`, `signature` | `{address}` |
//! | `sha256` | `text` | `{sha256}` |

use arena_core::{answer, merkle, payload, request, sha256_hex, task::Task};
use serde_json::{json, Value};

#[no_mangle]
pub extern "C" fn alloc(len: usize) -> *mut u8 {
    let mut buf = Vec::<u8>::with_capacity(len);
    let ptr = buf.as_mut_ptr();
    std::mem::forget(buf);
    ptr
}

/// # Safety
/// `ptr` and `len` come from [`alloc`] or from [`call`]'s answer.
#[no_mangle]
pub unsafe extern "C" fn dealloc(ptr: *mut u8, len: usize) {
    drop(Vec::from_raw_parts(ptr, 0, len));
}

/// Take a JSON request at `ptr`, return the answer as `(ptr << 32) | len`.
///
/// # Safety
/// `ptr` and `len` describe memory the caller wrote after [`alloc`].
#[no_mangle]
pub unsafe extern "C" fn call(ptr: *const u8, len: usize) -> u64 {
    let input = std::slice::from_raw_parts(ptr, len);
    let out = serde_json::from_slice::<Value>(input).map_err(|e| e.to_string()).and_then(|r| handle(&r));
    let text = match out {
        Ok(v) => v.to_string(),
        Err(e) => json!({ "error": e }).to_string(),
    }
    .into_bytes();
    let n = text.len();
    let p = alloc(n);
    std::ptr::copy_nonoverlapping(text.as_ptr(), p, n);
    ((p as u64) << 32) | n as u64
}

fn s<'a>(r: &'a Value, k: &str) -> Result<&'a str, String> {
    r.get(k).and_then(Value::as_str).ok_or_else(|| format!("`{k}` is missing"))
}

pub fn handle(r: &Value) -> Result<Value, String> {
    match s(r, "op")? {
        "leaf" => {
            let task = Task::from_json(s(r, "task")?.as_bytes())?;
            task.validate()?;
            Ok(json!({ "leaf": hex::encode(task.leaf()), "family": task.family() }))
        }
        "root" => {
            let leaves: Vec<merkle::Hash> = r["leaves"]
                .as_array()
                .ok_or("`leaves` is missing")?
                .iter()
                .map(|v| hex::decode(v.as_str().unwrap_or_default()).ok().and_then(|b| b.try_into().ok()).ok_or("a leaf is not 32 bytes of hex"))
                .collect::<Result<_, _>>()?;
            Ok(json!({ "root": merkle::root(&leaves).map(hex::encode) }))
        }
        "grade" => {
            let task = Task::from_json(s(r, "task")?.as_bytes())?;
            let g = answer::grade(&task, s(r, "response")?.as_bytes());
            Ok(json!({ "ok": g.verdict.ok, "reason": g.verdict.reason, "answer": g.answer }))
        }
        "request" => {
            let task = Task::from_json(s(r, "task")?.as_bytes())?;
            Ok(json!({ "sha256": sha256_hex(&request::participant(s(r, "model")?, &task)) }))
        }
        "nonce" => {
            let hashes: Vec<String> = r["hashes"].as_array().ok_or("`hashes` is missing")?.iter().map(|v| v.as_str().unwrap_or_default().to_string()).collect();
            Ok(json!({ "nonce": request::attestation_nonce(&hashes) }))
        }
        "judge" => {
            let key: [u8; 32] = hex::decode(s(r, "key")?).ok().and_then(|b| b.try_into().ok()).ok_or("the key is not 32 bytes of hex")?;
            let sig: [u8; 64] = hex::decode(s(r, "signature")?).ok().and_then(|b| b.try_into().ok()).ok_or("the signature is not 64 bytes of hex")?;
            let key = ed25519_dalek::VerifyingKey::from_bytes(&key).map_err(|_| "the key is not a curve point")?;
            let ok = key.verify_strict(&payload::signing_message(s(r, "payload")?), &ed25519_dalek::Signature::from_bytes(&sig)).is_ok();
            Ok(json!({ "ok": ok }))
        }
        "eip191" => Ok(json!({ "address": eip191(s(r, "text")?, s(r, "signature")?)? })),
        "sha256" => Ok(json!({ "sha256": sha256_hex(s(r, "text")?.as_bytes()) })),
        other => Err(format!("no op `{other}`")),
    }
}

fn eip191(text: &str, signature: &str) -> Result<String, String> {
    use k256::ecdsa::{RecoveryId, Signature, VerifyingKey};
    use sha3::{Digest, Keccak256};
    let raw = hex::decode(signature.trim_start_matches("0x")).map_err(|_| "the signature is not hex")?;
    if raw.len() != 65 {
        return Err("an EIP-191 signature is 65 bytes".into());
    }
    let sig = Signature::from_slice(&raw[..64]).map_err(|_| "r and s do not parse")?;
    let v = match raw[64] {
        v @ (27 | 28) => v - 27,
        v @ (0 | 1) => v,
        _ => return Err("bad recovery byte".into()),
    };
    let mut h = Keccak256::new();
    h.update(format!("\x19Ethereum Signed Message:\n{}", text.len()).as_bytes());
    h.update(text.as_bytes());
    let digest: [u8; 32] = h.finalize().into();
    let key = VerifyingKey::recover_from_prehash(&digest, &sig, RecoveryId::from_byte(v).ok_or("bad recovery id")?).map_err(|_| "no key recovers")?;
    let point = key.to_encoded_point(false);
    Ok(format!("0x{}", hex::encode(&Keccak256::digest(&point.as_bytes()[1..])[12..])))
}
