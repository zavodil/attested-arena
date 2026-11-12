//! Attested Arena's judge, an OutLayer module that runs in an Intel TDX
//! enclave.
//!
//! | `op` | from | does |
//! |---|---|---|
//! | `identity` | anyone | prints the judge's public key, signed by it |
//! | `generate` | the arena | a model in NEAR AI's TEE writes tasks; code admits the sound ones; each is sealed to this build and only leaves the enclave as ciphertext and a leaf hash |
//! | `portion` | the arena | opens the arena's next tasks, has the champion and the challenger answer them in NEAR AI's TEEs, checks every answer's attestation and signature, scores the answers with code, and reveals the tasks with the result |
//! | `reveal` | the arena, after the season | opens tasks no duel used |
//!
//! **Keys bound to this build.** The judge runs directly from its wasm URL,
//! never as a project, so its keys are derived from the wasm's sha256
//! (`bind: "wasm"`). Any other build — a later version of this code too —
//! gets other keys: it can neither open the pool nor sign as this judge.
//!
//! * `judge` (ed25519, no caller): signs every result. The arena pins its
//!   public key when it is created.
//! * `pool` (the transaction's signer): seals the tasks. Only this build, run
//!   in a transaction the operator signed, opens them.
//!
//! **Only through the arena.** Every operation but `identity` checks that
//! the run's predecessor is the arena named in the input and that the arena's
//! account has no full-access key, so its code — which decides which tasks a
//! portion opens — cannot change. A direct call by the operator, or by
//! anyone, opens nothing.

mod chain;
mod eip191;
mod evidence;
mod http;
mod intel;
mod nearai;
mod nvidia;

use arena_core::answer::{self, Graded};
use arena_core::families::{lines, seating, Rejection};
use arena_core::payload::{self, Batch, Identity, JudgeOutput, Outcome, Payload, Phase, Portion, Reveal};
use arena_core::request;
use arena_core::sha256_hex;
use arena_core::task::Task;
use arena_core::Rng;
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use evidence::Attested;
use nearai::{Client, Exchange};
use serde::Deserialize;
use serde_json::json;
use std::collections::BTreeMap;

/// The manifest, embedded so the build's sha256 covers it: the build-bound
/// keys and the hosts the enclave reaches.
#[cfg(target_family = "wasm")]
#[used]
#[link_section = "outlayer.manifest"]
static OUTLAYER_MANIFEST: [u8; include_bytes!("../manifest.json").len()] = *include_bytes!("../manifest.json");

const SIGNING_KEY: &str = "judge";
const POOL_KEY: &str = "pool";
/// The most tasks one `generate` asks a model for.
const MAX_BATCH: u32 = 8;
const GENERATOR_MAX_TOKENS: u32 = 8000;
const READER_MAX_TOKENS: u32 = 3000;

#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
enum Input {
    Identity,
    Generate { arena: String, batch: u32, family: String, count: u32, generator: String, reader: String },
    Portion { arena: String, duel: u32, portion: u32, phase: Phase, champion: String, challenger: String, tasks: Vec<Sealed> },
    Reveal { arena: String, tasks: Vec<Sealed> },
}

/// A task as the arena holds it.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Sealed {
    leaf: String,
    /// Base64.
    ciphertext: String,
}

fn main() {
    let result = serde_json::from_slice::<Input>(&outlayer::env::input())
        .map_err(|e| format!("the input does not read: {e}"))
        .and_then(run);
    match result {
        Ok(out) => outlayer::env::output_json(&out).expect("the result always serialises"),
        Err(error) => outlayer::env::output_json(&json!({ "error": error })).expect("an error always serialises"),
    }
}

fn run(input: Input) -> Result<JudgeOutput, String> {
    match input {
        Input::Identity => {
            let key = outlayer::signing_keys::public_key(SIGNING_KEY, None).map_err(|e| format!("the judge's key is not available: {e}"))?;
            sign(&Payload::Identity(Identity { judge_key: hex::encode(key) }), json!({}))
        }
        Input::Generate { arena, batch, family, count, generator, reader } => generate(arena, batch, &family, count, generator, reader),
        Input::Portion { arena, duel, portion, phase, champion, challenger, tasks } => {
            judge_portion(arena, duel, portion, phase, champion, challenger, tasks)
        }
        Input::Reveal { arena, tasks } => reveal(arena, tasks),
    }
}

/// The run must come from the arena itself, and the arena must be locked.
/// Returns the operator: the account that signed the transaction, as the host
/// says it.
fn guard(arena: &str) -> Result<String, String> {
    let predecessor = outlayer::env::predecessor_account_id().unwrap_or_default();
    if predecessor != arena {
        return Err(format!("this operation is taken only from the arena {arena} itself, not from `{predecessor}`"));
    }
    chain::assert_locked(arena)?;
    std::env::var("NEAR_USER_ACCOUNT_ID")
        .ok()
        .filter(|a| !a.is_empty())
        .ok_or_else(|| "the host named no signer for this run".to_string())
}

fn sign(payload: &Payload, evidence: serde_json::Value) -> Result<JudgeOutput, String> {
    let payload = serde_json::to_string(payload).map_err(|e| e.to_string())?;
    let signature = outlayer::signing_keys::sign(SIGNING_KEY, None, &payload::signing_message(&payload))
        .map_err(|e| format!("the judge could not sign: {e}"))?;
    let evidence = serde_json::to_string(&evidence).map_err(|e| e.to_string())?;
    Ok(JudgeOutput { payload, signature: hex::encode(signature), evidence })
}

fn aad(arena: &str, leaf: &str) -> Vec<u8> {
    format!("attested-arena/task:{arena}:{leaf}").into_bytes()
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn random_bytes<const N: usize>() -> Result<[u8; N], String> {
    let mut bytes = [0u8; N];
    getrandom::getrandom(&mut bytes).map_err(|e| format!("no randomness: {e}"))?;
    Ok(bytes)
}

/// One model's exchanges in a run, every one proven: attested once for the
/// hash of all its requests, then each answer's signature checked.
struct Proven {
    attested: Attested,
    exchanges: Vec<Exchange>,
    signatures: Vec<nearai::ResponseSignature>,
}

fn ask(api: &Client, model: &str, bodies: Vec<Vec<u8>>, now: u64) -> Result<Proven, String> {
    Ok(ask_all(api, vec![(model, bodies)], now)?.remove(0))
}

/// Several models' exchanges at once: each model is attested for its own
/// requests, then every chat goes out together, then every signature.
fn ask_all(api: &Client, work: Vec<(&str, Vec<Vec<u8>>)>, now: u64) -> Result<Vec<Proven>, String> {
    // Each model's report, for the hash of its requests, before anything is sent.
    let mut nonces = Vec::new();
    let mut reports = Vec::new();
    for (model, bodies) in &work {
        let hashes: Vec<String> = bodies.iter().map(|b| sha256_hex(b)).collect();
        let nonce = request::attestation_nonce(&hashes);
        reports.push(evidence::reports(api, model, &nonce)?);
        nonces.push(nonce);
    }
    let sizes: Vec<usize> = work.iter().map(|(_, b)| b.len()).collect();
    let names: Vec<String> = work.iter().map(|(m, _)| m.to_string()).collect();
    let models: Vec<&str> = work.iter().flat_map(|(m, b)| std::iter::repeat_n(*m, b.len())).collect();
    let all: Vec<Vec<u8>> = work.into_iter().flat_map(|(_, b)| b).collect();
    let exchanges: Vec<Exchange> = api.chats(all).into_iter().collect::<Result<_, _>>()?;
    let ids: Vec<(&str, &str)> = exchanges.iter().zip(&models).map(|(e, m)| (e.id.as_str(), *m)).collect();
    let signatures: Vec<_> = api.signatures(&ids).into_iter().collect::<Result<_, _>>()?;
    let mut exchanges = exchanges.into_iter();
    let mut signatures = signatures.into_iter();
    let mut out = Vec::new();
    for (((model, nonce), reports), size) in names.iter().zip(nonces).zip(reports).zip(sizes) {
        let exchanges: Vec<Exchange> = exchanges.by_ref().take(size).collect();
        let signatures: Vec<_> = signatures.by_ref().take(size).collect();
        let signers: Vec<String> = exchanges.iter().zip(&signatures).map(|(e, s)| evidence::signer(model, e, s)).collect::<Result<_, _>>()?;
        let attested = evidence::prove(model, &nonce, reports, &signers, now)?;
        out.push(Proven { attested, exchanges, signatures });
    }
    Ok(out)
}

fn generate(arena: String, batch: u32, family: &str, count: u32, generator: String, reader: String) -> Result<JudgeOutput, String> {
    let operator = guard(&arena)?;
    if !(1..=MAX_BATCH).contains(&count) {
        return Err(format!("a batch asks for 1 to {MAX_BATCH} tasks"));
    }
    let api = Client::from_env()?;
    let now = now();
    let mut rng = Rng::new(u64::from_le_bytes(random_bytes::<8>()?));
    let mut rejected: BTreeMap<String, u32> = BTreeMap::new();
    let mut reject = |r: Rejection| *rejected.entry(r.code().to_string()).or_default() += 1;
    let mut admitted: Vec<Task> = Vec::new();
    let (wrote, readers_evidence, generated) = match family {
        "seating" => {
            // Code designs each puzzle here, from the enclave's randomness;
            // the model only tells it as a story.
            let plans: Vec<seating::Plan> = (0..count as usize).map(|i| seating::design(&mut rng, 6 + i % 2)).collect();
            let wrote = ask(&api, &generator, vec![request::chat(&generator, &seating::story_prompt(&plans), GENERATOR_MAX_TOKENS)], now)?;
            let stories = seating::parse_stories(&answer::content(&wrote.exchanges[0].response)?)?;
            let generated = plans.len() as u32;
            let candidates: Vec<seating::Candidate> = plans.into_iter().zip(stories).map(|(p, s)| p.with_story(s)).collect();
            for _ in candidates.len()..generated as usize {
                reject(Rejection::Malformed("the writer told fewer stories than there were puzzles".to_string()));
            }
            let bodies = candidates.iter().map(|c| request::chat(&reader, &seating::reader_prompt(&c.story), READER_MAX_TOKENS)).collect();
            let read = ask(&api, &reader, bodies, now)?;
            for (candidate, exchange) in candidates.into_iter().zip(&read.exchanges) {
                let reading = answer::content(&exchange.response).ok().and_then(|t| seating::parse_reading(&t).ok());
                match seating::admit(candidate, reading, hex::encode(random_bytes::<32>()?)) {
                    Ok(task) => admitted.push(Task::Seating(task)),
                    Err(r) => reject(r),
                }
            }
            let readers = summary(&reader, &read);
            (wrote, Some(readers), generated)
        }
        "lines" => {
            let wrote = ask(&api, &generator, vec![request::chat(&generator, &lines::generator_prompt(count as usize), GENERATOR_MAX_TOKENS)], now)?;
            let candidates = lines::parse_candidates(&answer::content(&wrote.exchanges[0].response)?)?;
            let generated = candidates.len() as u32;
            for c in candidates {
                let salt = hex::encode(random_bytes::<32>()?);
                let outcome = serde_json::from_value::<lines::Candidate>(c)
                    .map_err(|e| Rejection::Malformed(e.to_string()))
                    .and_then(|c| lines::admit(c, &mut rng, salt));
                match outcome {
                    Ok(task) => admitted.push(Task::Lines(task)),
                    Err(r) => reject(r),
                }
            }
            (wrote, None, generated)
        }
        other => return Err(format!("there is no family `{other}`")),
    };

    let mut leaves = Vec::new();
    let mut ciphertexts = Vec::new();
    let mut sealed = Vec::new();
    for task in &admitted {
        let leaf = hex::encode(task.leaf());
        let ct = outlayer::encryption_keys::encrypt(POOL_KEY, None, &task.to_json(), &aad(&arena, &leaf))
            .map_err(|e| format!("the task could not be sealed: {e}"))?;
        ciphertexts.push(sha256_hex(&ct));
        sealed.push(BASE64.encode(&ct));
        leaves.push(leaf);
    }
    let evidence = json!({
        "ciphertexts": sealed,
        "generator": summary(&generator, &wrote),
        "reader": readers_evidence,
    });
    let evidence_sha256 = sha256_hex(serde_json::to_string(&evidence).map_err(|e| e.to_string())?.as_bytes());
    let payload = Payload::Batch(Batch {
        arena,
        operator,
        batch,
        family: family.to_string(),
        leaves,
        ciphertexts,
        generated,
        rejected,
        evidence_sha256,
    });
    sign(&payload, evidence)
}

/// What a generation exchange leaves in the evidence: the hashes and the
/// proof it ran in a TEE, never the text — the text holds the tasks.
fn summary(model: &str, proven: &Proven) -> serde_json::Value {
    let exchanges: Vec<_> = proven
        .exchanges
        .iter()
        .zip(&proven.signatures)
        .map(|(e, s)| json!({ "request_sha256": sha256_hex(&e.request), "response_sha256": sha256_hex(&e.response), "signature": s }))
        .collect();
    json!({ "model": model, "attestation": proven.attested, "exchanges": exchanges })
}

/// Open the arena's tasks: each ciphertext opens only under its own leaf,
/// and the plaintext must hash to that leaf.
fn open(arena: &str, tasks: &[Sealed]) -> Result<Vec<(String, Task, Vec<u8>)>, String> {
    tasks
        .iter()
        .map(|s| {
            let ct = BASE64.decode(&s.ciphertext).map_err(|_| format!("the ciphertext of {} is not base64", s.leaf))?;
            let json = outlayer::encryption_keys::decrypt(POOL_KEY, None, &ct, &aad(arena, &s.leaf))
                .map_err(|e| format!("the task {} does not open with this build's key: {e}", s.leaf))?;
            let task = Task::from_json(&json)?;
            if hex::encode(task.leaf()) != s.leaf {
                return Err(format!("the task opened under {} hashes to another leaf", s.leaf));
            }
            task.validate()?;
            Ok((s.leaf.clone(), task, json))
        })
        .collect()
}

fn judge_portion(
    arena: String,
    duel: u32,
    portion: u32,
    phase: Phase,
    champion: String,
    challenger: String,
    tasks: Vec<Sealed>,
) -> Result<JudgeOutput, String> {
    let operator = guard(&arena)?;
    if tasks.is_empty() {
        return Err("a portion has at least one task".to_string());
    }
    let opened = open(&arena, &tasks)?;
    let api = Client::from_env()?;
    let now = now();

    let bodies = |model: &str| opened.iter().map(|(_, task, _)| request::participant(model, task)).collect::<Vec<_>>();
    let mut proven = ask_all(&api, vec![(champion.as_str(), bodies(&champion)), (challenger.as_str(), bodies(&challenger))], now)?;
    let (chall, champ) = (proven.pop().expect("two models asked"), proven.pop().expect("two models asked"));

    let mut results = Vec::new();
    let mut answers = Vec::new();
    for (i, (_, task, _)) in opened.iter().enumerate() {
        let a: Graded = answer::grade(task, &champ.exchanges[i].response);
        let b: Graded = answer::grade(task, &chall.exchanges[i].response);
        results.push(Outcome { champion: a.verdict.ok, challenger: b.verdict.ok });
        for (model, proven, graded) in [(&champion, &champ, a), (&challenger, &chall, b)] {
            answers.push(json!({
                "task": i,
                "model": model,
                "response": String::from_utf8_lossy(&proven.exchanges[i].response),
                "signature": proven.signatures[i],
                "verdict": graded.verdict,
            }));
        }
    }
    let evidence = json!({
        "tasks": opened.iter().map(|(_, _, json)| String::from_utf8_lossy(json).into_owned()).collect::<Vec<_>>(),
        "answers": answers,
        "attestations": [champ.attested, chall.attested],
    });
    let evidence_sha256 = sha256_hex(serde_json::to_string(&evidence).map_err(|e| e.to_string())?.as_bytes());
    let payload = Payload::Portion(Portion {
        arena,
        operator,
        duel,
        portion,
        phase,
        champion,
        challenger,
        leaves: opened.iter().map(|(leaf, _, _)| leaf.clone()).collect(),
        results,
        evidence_sha256,
    });
    sign(&payload, evidence)
}

fn reveal(arena: String, tasks: Vec<Sealed>) -> Result<JudgeOutput, String> {
    let operator = guard(&arena)?;
    let opened = open(&arena, &tasks)?;
    let evidence = json!({ "tasks": opened.iter().map(|(_, _, json)| String::from_utf8_lossy(json).into_owned()).collect::<Vec<_>>() });
    let evidence_sha256 = sha256_hex(serde_json::to_string(&evidence).map_err(|e| e.to_string())?.as_bytes());
    let payload = Payload::Reveal(Reveal { arena, operator, leaves: opened.into_iter().map(|(leaf, _, _)| leaf).collect(), evidence_sha256 });
    sign(&payload, evidence)
}
