//! `arena-verify` re-checks an Attested Arena season from NEAR state alone.
//!
//! ```text
//! arena-verify --arena season1.attested-arena.testnet [--rpc URL] [--export DIR]
//! arena-verify --snapshot DIR
//! ```
//!
//! It trusts no one's word — not the operator's, not the judge's, not this
//! repository's report:
//!
//! 1. **Pool.** The Merkle root over the on-chain leaves is the root the
//!    contract fixed at `seal`.
//! 2. **Judge.** Every batch, portion and reveal is signed by the build's key
//!    the contract pinned; every payload names this arena; every evidence blob
//!    hashes to what its payload says.
//! 3. **Tasks.** Every revealed task is canonical JSON whose leaf is the one
//!    its portion was assigned, proven into the root, and well formed.
//! 4. **Answers.** For every answer, the exact request is rebuilt from the
//!    task and the model; the model's TEE signature covers that request and
//!    the response byte for byte; its signer is an instance whose Intel TDX
//!    quote verifies against Intel's collateral and binds that signer to the
//!    hash of the run's requests.
//! 5. **Scores.** Every answer is scored again with the same checker code; the
//!    result must equal what the judge signed.
//! 6. **Duels and money.** Each duel's sequential test is replayed from the
//!    signed results; its outcome and the tokens minted must match the chain.
//!
//! `--export DIR` saves every view it read; `--snapshot DIR` checks those
//! files offline (Intel's collateral is still fetched unless `--no-intel`).

mod eip191;
mod intel;

use arena_core::answer;
use arena_core::merkle;
use arena_core::payload::{signing_message, Payload};
use arena_core::request;
use arena_core::sha256_hex;
use arena_core::sprt::Sprt;
use arena_core::task::Task;
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;

// ---- where state comes from -----------------------------------------------

enum Source {
    Rpc { url: String, arena: String, http: reqwest::blocking::Client, export: Option<PathBuf> },
    Snapshot(PathBuf),
}

impl Source {
    fn file(method: &str, args: &Value) -> String {
        let a = args.to_string();
        if a == "{}" {
            format!("{method}.json")
        } else {
            format!("{method}-{}.json", &sha256_hex(a.as_bytes())[..16])
        }
    }

    fn view(&self, method: &str, args: Value) -> Result<Value, String> {
        match self {
            Source::Snapshot(dir) => {
                let path = dir.join(Self::file(method, &args));
                let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
                serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
            }
            Source::Rpc { url, arena, http, export } => {
                let body = json!({
                    "jsonrpc": "2.0", "id": "arena-verify", "method": "query",
                    "params": { "request_type": "call_function", "finality": "final", "account_id": arena,
                                "method_name": method, "args_base64": BASE64.encode(args.to_string()) }
                });
                let mut last = String::new();
                for attempt in 0..4 {
                    if attempt > 0 {
                        std::thread::sleep(std::time::Duration::from_secs(2 * attempt));
                    }
                    let reply: Value = match http.post(url).json(&body).send().and_then(|r| r.json()) {
                        Ok(v) => v,
                        Err(e) => {
                            last = e.to_string();
                            continue;
                        }
                    };
                    if let Some(e) = reply.get("error") {
                        last = e.to_string();
                        continue;
                    }
                    let bytes: Vec<u8> = reply
                        .pointer("/result/result")
                        .and_then(Value::as_array)
                        .ok_or_else(|| format!("{method}: the RPC answer has no result: {reply}"))?
                        .iter()
                        .map(|b| b.as_u64().unwrap_or(0) as u8)
                        .collect();
                    let value: Value = serde_json::from_slice(&bytes).map_err(|e| format!("{method}: {e}"))?;
                    if let Some(dir) = export {
                        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
                        std::fs::write(dir.join(Self::file(method, &args)), serde_json::to_string_pretty(&value).unwrap())
                            .map_err(|e| e.to_string())?;
                    }
                    return Ok(value);
                }
                Err(format!("{method}: {last}"))
            }
        }
    }

    fn paged(&self, method: &str, total: u64, page: u64) -> Result<Vec<Value>, String> {
        let mut out = Vec::new();
        let mut from = 0;
        while from < total {
            let got = self.view(method, json!({ "from": from, "limit": page }))?;
            out.extend(got.as_array().cloned().unwrap_or_default());
            from += page;
        }
        Ok(out)
    }
}

// ---- the ledger of checks -----------------------------------------------------

#[derive(Default)]
struct Ledger {
    passed: usize,
    broken: Vec<String>,
}

impl Ledger {
    fn check(&mut self, what: impl AsRef<str>, result: Result<(), String>) {
        match result {
            Ok(()) => self.passed += 1,
            Err(why) => {
                println!("  BROKEN  {}: {why}", what.as_ref());
                self.broken.push(format!("{}: {why}", what.as_ref()));
            }
        }
    }
}

fn ensure(cond: bool, why: impl Into<String>) -> Result<(), String> {
    if cond {
        Ok(())
    } else {
        Err(why.into())
    }
}

// ---- the checks ---------------------------------------------------------------

struct Season {
    judge_key: VerifyingKey,
    arena: String,
    owner: String,
    root: [u8; 32],
    leaves: Vec<[u8; 32]>,
    config: Value,
}

fn verify_signed(season: &Season, signed: &Value) -> Result<(Payload, String), String> {
    let payload = signed["payload"].as_str().ok_or("no payload")?;
    let sig = hex::decode(signed["signature"].as_str().ok_or("no signature")?).map_err(|_| "signature not hex")?;
    let sig = Signature::from_slice(&sig).map_err(|_| "signature not 64 bytes")?;
    season.judge_key.verify(&signing_message(payload), &sig).map_err(|_| "not signed by the pinned judge build".to_string())?;
    let parsed: Payload = serde_json::from_str(payload).map_err(|e| format!("payload: {e}"))?;
    let (arena, operator) = match &parsed {
        Payload::Batch(b) => (&b.arena, &b.operator),
        Payload::Portion(p) => (&p.arena, &p.operator),
        Payload::Reveal(r) => (&r.arena, &r.operator),
        Payload::Identity(_) => return Err("an identity is not a result".into()),
    };
    ensure(*arena == season.arena, format!("names the arena {arena}"))?;
    ensure(*operator == season.owner, format!("names the operator {operator}"))?;
    let digest = signed["evidence_sha256"].as_str().unwrap_or_default().to_string();
    Ok((parsed, digest))
}

/// An attested model in a run's evidence: its proven signers, after this tool
/// verified each instance's quote with Intel and its binding to `nonce`.
fn proven_signers(attested: &Value, nonce: &str, intel: &mut Option<intel::Intel>, now: u64) -> Result<Vec<String>, String> {
    ensure(attested["nonce"].as_str() == Some(nonce), "the attestation was not taken for this run's requests")?;
    let mut signers = Vec::new();
    for inst in attested["instances"].as_array().cloned().unwrap_or_default() {
        let address = inst["signing_address"].as_str().unwrap_or_default().to_ascii_lowercase();
        let quote = hex::decode(inst["intel_quote"].as_str().unwrap_or_default()).map_err(|_| "quote not hex")?;
        let report_data = match intel.as_mut() {
            Some(i) => match i.verify(&quote, now) {
                Ok(v) => v.report_data,
                Err(e) => return Err(format!("{address}: Intel does not verify the quote: {e}")),
            },
            None => {
                let parsed = dcap_qvl::quote::Quote::parse(&quote).map_err(|e| format!("quote: {e}"))?;
                hex::encode(parsed.report.as_td10().ok_or("not a TD report")?.report_data)
            }
        };
        let mut expected = hex::decode(address.trim_start_matches("0x")).map_err(|_| "address not hex")?;
        expected.resize(32, 0);
        expected.extend(hex::decode(nonce).map_err(|_| "nonce not hex")?);
        if hex::encode(&expected) != report_data {
            return Err(format!("{address}: the quote does not bind this signer to this run's requests"));
        }
        if inst["gpu"]["ok"].as_bool() != Some(true) {
            continue;
        }
        signers.push(address);
    }
    ensure(!signers.is_empty(), "no instance is proven")?;
    Ok(signers)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let flag = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
    let no_intel = args.iter().any(|a| a == "--no-intel");
    let source = if let Some(dir) = flag("--snapshot") {
        Source::Snapshot(PathBuf::from(dir))
    } else {
        let Some(arena) = flag("--arena") else {
            eprintln!("usage: arena-verify --arena <account> [--rpc URL] [--export DIR] [--no-intel] | --snapshot DIR [--no-intel]");
            std::process::exit(2);
        };
        Source::Rpc {
            url: flag("--rpc").unwrap_or_else(|| "https://rpc.testnet.near.org".into()),
            arena,
            http: reqwest::blocking::Client::new(),
            export: flag("--export").map(PathBuf::from),
        }
    };
    match run(&source, !no_intel) {
        Ok(ledger) => {
            println!("\n{} checks passed, {} broken.", ledger.passed, ledger.broken.len());
            std::process::exit(if ledger.broken.is_empty() { 0 } else { 1 });
        }
        Err(e) => {
            eprintln!("arena-verify could not read the season: {e}");
            std::process::exit(2);
        }
    }
}

fn run(src: &Source, with_intel: bool) -> Result<Ledger, String> {
    let mut l = Ledger::default();
    let mut intel = with_intel.then(intel::Intel::new);
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
    let s = src.view("get_season", json!({}))?;
    let count = |k: &str| s[k].as_u64().unwrap_or(0);
    let key_bytes: [u8; 32] = hex::decode(s["judge"]["key"].as_str().unwrap_or_default())
        .ok()
        .and_then(|b| b.try_into().ok())
        .ok_or("the pinned judge key is not 32 bytes")?;
    let arena = match src {
        Source::Rpc { arena, .. } => arena.clone(),
        Source::Snapshot(_) => s["arena"].as_str().map(str::to_string).unwrap_or_default(),
    };
    println!("Season {arena}: {} — judge build {}", s["status"], s["judge"]["wasm_sha256"].as_str().unwrap_or("?"));
    println!("  judge key {}  ({})", s["judge"]["key"].as_str().unwrap_or("?"), s["judge"]["wasm_url"].as_str().unwrap_or("?"));

    // 1. Pool.
    println!("\n1. The pool");
    let leaves: Vec<[u8; 32]> = src
        .paged("get_leaves", count("pool_tasks"), 200)?
        .iter()
        .map(|v| hex::decode(v.as_str().unwrap_or_default()).ok().and_then(|b| b.try_into().ok()).unwrap_or([0; 32]))
        .collect();
    let root: [u8; 32] = s["root"].as_str().and_then(|r| hex::decode(r).ok()).and_then(|b| b.try_into().ok()).unwrap_or([0; 32]);
    l.check("the Merkle root over the on-chain leaves is the root fixed at seal", ensure(merkle::root(&leaves) == Some(root), "the roots differ"));
    println!("  {} tasks, root {}", leaves.len(), hex::encode(root));
    let season = Season {
        judge_key: VerifyingKey::from_bytes(&key_bytes).map_err(|_| "the judge key is not a point")?,
        arena: arena.clone(),
        owner: s["owner"].as_str().unwrap_or_default().to_string(),
        root,
        leaves,
        config: s["config"].clone(),
    };
    let sprt = Sprt::arena();
    let cfg = &season.config["sprt"];
    l.check(
        "the season's test is SPRT(p1 = 0.7, α = 5%, β = 10%)",
        ensure(
            cfg["win"].as_i64() == Some(sprt.win) && cfg["loss"].as_i64() == Some(sprt.loss) && cfg["accept"].as_i64() == Some(sprt.accept) && cfg["reject"].as_i64() == Some(sprt.reject),
            "other parameters",
        ),
    );

    // 2. Batches.
    println!("\n2. Batches — generated inside the TEE, admitted by code");
    let mut generated = 0;
    let mut admitted = 0;
    let mut rejected: HashMap<String, u64> = HashMap::new();
    for (i, b) in src.paged("get_batches", count("batches"), 50)?.iter().enumerate() {
        let Some(signed) = b["state"].get("Done") else { continue };
        let first = b["first_leaf"].as_u64().unwrap_or(0) as usize;
        let res = (|| {
            let (payload, digest) = verify_signed(&season, signed)?;
            let Payload::Batch(p) = payload else { return Err("not a batch".to_string()) };
            let on_chain: Vec<String> = season.leaves[first..first + p.leaves.len()].iter().map(hex::encode).collect();
            ensure(p.leaves == on_chain, "its leaves are not the ones on chain")?;
            generated += p.generated as u64;
            admitted += p.leaves.len() as u64;
            for (k, v) in &p.rejected {
                *rejected.entry(k.clone()).or_default() += *v as u64;
            }
            if let Some(ev) = src.view("get_evidence", json!({ "key": format!("b{i}") }))?.as_str() {
                ensure(sha256_hex(ev.as_bytes()) == digest, "the stored evidence is not the signed evidence")?;
                let ev: Value = serde_json::from_str(ev).map_err(|e| e.to_string())?;
                for (j, ct) in ev["ciphertexts"].as_array().cloned().unwrap_or_default().iter().enumerate() {
                    let bytes = BASE64.decode(ct.as_str().unwrap_or_default()).map_err(|_| "ciphertext not base64")?;
                    ensure(sha256_hex(&bytes) == p.ciphertexts[j], "a ciphertext is not the signed one")?;
                }
                let g = &ev["generator"];
                let model = g["model"].as_str().unwrap_or_default();
                let req = g["exchanges"][0]["request_sha256"].as_str().unwrap_or_default().to_string();
                let signers = proven_signers(&g["attestation"], &request::attestation_nonce(std::slice::from_ref(&req)), &mut intel, now)?;
                let sig = &g["exchanges"][0]["signature"];
                let text = sig["text"].as_str().unwrap_or_default();
                ensure(text.starts_with(&format!("{model}:{req}:")), "the generator's signature is over another request")?;
                let signer = eip191::recover(text, sig["signature"].as_str().unwrap_or_default())?;
                ensure(signers.contains(&signer), "the generator's answer is not signed by a proven instance")?;
            }
            Ok(())
        })();
        l.check(format!("batch {i}"), res);
    }
    println!("  generated {generated}, admitted {admitted}; kept out: {rejected:?}");

    // 3–5. Portions.
    println!("\n3. Portions — every task, answer, signature and score");
    let portions = src.paged("get_portions", count("portions"), 20)?;
    let mut tasks_checked = 0;
    let mut answers_checked = 0;
    for (i, rec) in portions.iter().enumerate() {
        let Some(signed) = rec["state"].get("Done") else {
            if let Some(why) = rec["state"].get("Failed") {
                println!("  portion {i}: failed, its {} tasks spent unopened ({})", rec["count"], why.as_str().unwrap_or(""));
            }
            continue;
        };
        let res = (|| {
            let (payload, digest) = verify_signed(&season, signed)?;
            let Payload::Portion(p) = payload else { return Err("not a portion".to_string()) };
            let first = rec["first_task"].as_u64().unwrap_or(0) as usize;
            let n = rec["count"].as_u64().unwrap_or(0) as usize;
            let assigned: Vec<String> = season.leaves[first..first + n].iter().map(hex::encode).collect();
            ensure(p.leaves == assigned, "the signed tasks are not the ones assigned")?;
            let recorded: Vec<(bool, bool)> = serde_json::from_value(rec["results"].clone()).map_err(|e| e.to_string())?;
            ensure(p.results.iter().map(|o| (o.champion, o.challenger)).collect::<Vec<_>>() == recorded, "the chain records other results")?;
            let ev = src.view("get_evidence", json!({ "key": format!("p{i}") }))?;
            let Some(ev) = ev.as_str() else { return Err("its evidence was pruned".to_string()) };
            ensure(sha256_hex(ev.as_bytes()) == digest, "the stored evidence is not the signed evidence")?;
            let ev: Value = serde_json::from_str(ev).map_err(|e| e.to_string())?;

            let mut tasks = Vec::new();
            for (j, t) in ev["tasks"].as_array().cloned().unwrap_or_default().iter().enumerate() {
                let task = Task::from_json(t.as_str().unwrap_or_default().as_bytes())?;
                ensure(hex::encode(task.leaf()) == p.leaves[j], format!("task {} does not hash to its leaf", first + j))?;
                let proof = merkle::proof(&season.leaves, first + j);
                ensure(merkle::verify(&task.leaf(), first + j, season.leaves.len(), &proof, &season.root), "a task does not prove into the root")?;
                task.validate()?;
                tasks.push(task);
                tasks_checked += 1;
            }
            ensure(tasks.len() == n, "not every task is revealed")?;

            // The two models' request hashes, in task order: the nonce each was attested for.
            let models = [p.champion.as_str(), p.challenger.as_str()];
            let mut signers = Vec::new();
            for (k, model) in models.iter().enumerate() {
                let hashes: Vec<String> = tasks.iter().map(|t| sha256_hex(&request::participant(model, t))).collect();
                signers.push(proven_signers(&ev["attestations"][k], &request::attestation_nonce(&hashes), &mut intel, now)?);
            }
            // The judge records each task's two answers in order: the
            // champion's, then the challenger's. The order — not the model
            // name — says whose an answer is, so a duel between two entries
            // of the same model is read right.
            let list = ev["answers"].as_array().cloned().unwrap_or_default();
            ensure(list.len() == 2 * tasks.len(), "not every task has both answers")?;
            for (n, a) in list.into_iter().enumerate() {
                let j = a["task"].as_u64().ok_or("an answer names no task")? as usize;
                ensure(j == n / 2, "the answers are out of order")?;
                let k = n % 2;
                let model = a["model"].as_str().unwrap_or_default();
                ensure(model == models[k], "an answer by another model than its side's")?;
                let response = a["response"].as_str().unwrap_or_default().as_bytes();
                let req = sha256_hex(&request::participant(model, &tasks[j]));
                let text = format!("{model}:{req}:{}", sha256_hex(response));
                let sig = &a["signature"];
                ensure(sig["signature_kind"].as_str() == Some("provider_tee"), "an answer is not signed in the model's TEE")?;
                ensure(sig["text"].as_str() == Some(text.as_str()), "a signature is not over this exact request and response")?;
                let signer = eip191::recover(&text, sig["signature"].as_str().unwrap_or_default())?;
                ensure(signers[k].contains(&signer), "an answer is signed by an unproven key")?;
                let graded = answer::grade(&tasks[j], response);
                let signed_ok = if k == 0 { p.results[j].champion } else { p.results[j].challenger };
                ensure(graded.verdict.ok == signed_ok, format!("task {}: the checker says {} for {model}, the judge signed {signed_ok}", first + j, graded.verdict.ok))?;
                answers_checked += 1;
            }
            Ok(())
        })();
        l.check(format!("portion {i}"), res);
    }
    println!("  {tasks_checked} tasks opened and proven into the root, {answers_checked} answers re-scored");

    // 6. Duels.
    println!("\n4. Duels — the test replayed from the signed results");
    let max_portions = season.config["max_portions"].as_u64().unwrap_or(0) as u32;
    let mut minted: u64 = 0;
    for (d, duel) in src.paged("get_duels", count("duels"), 50)?.iter().enumerate() {
        let mut phase_confirm = false;
        let (mut llr, mut in_phase, mut ct, mut cc, mut cx) = (0i64, 0u32, 0u32, 0u32, 0u32);
        let mut outcome: Option<Value> = None;
        for id in duel["portions"].as_array().cloned().unwrap_or_default() {
            let rec = &portions[id.as_u64().unwrap_or(0) as usize];
            if rec["state"].get("Done").is_none() || outcome.is_some() {
                continue;
            }
            let results: Vec<(bool, bool)> = serde_json::from_value(rec["results"].clone()).unwrap_or_default();
            for (c, x) in results {
                llr = sprt.step(llr, c, x);
                if phase_confirm {
                    ct += 1;
                    cc += c as u32;
                    cx += x as u32;
                }
            }
            in_phase += 1;
            let accept = llr >= sprt.accept;
            let reject = llr <= sprt.reject;
            let out = in_phase >= max_portions;
            if !phase_confirm {
                if accept {
                    phase_confirm = true;
                    llr = 0;
                    in_phase = 0;
                } else if reject {
                    outcome = Some(json!("Rejected"));
                } else if out {
                    outcome = Some(json!("NotProven"));
                }
            } else if accept {
                let gain = cx.saturating_sub(cc) * 10_000 / ct.max(1);
                minted += gain as u64;
                outcome = Some(json!({ "Record": { "gain_centi": gain } }));
            } else if reject || out {
                outcome = Some(json!("NotConfirmed"));
            }
        }
        let chain = duel["outcome"].clone();
        println!(
            "  duel {d}: {} vs champion {} → {}",
            duel["challenger_model"].as_str().unwrap_or("?"),
            duel["champion_model"].as_str().unwrap_or("?"),
            if chain.is_null() { "running".to_string() } else { chain.to_string() }
        );
        // `end_duel`: a duel the pool could not finish ends undecided.
        let pool_left = s["pool_tasks"].as_u64().unwrap_or(0).saturating_sub(s["next_task"].as_u64().unwrap_or(0));
        if outcome.is_none() && chain == "NotProven" && pool_left < season.config["portion_size"].as_u64().unwrap_or(0) {
            outcome = Some(json!("NotProven"));
        }
        if !chain.is_null() || outcome.is_some() {
            l.check(format!("duel {d}'s outcome"), ensure(outcome.as_ref() == Some(&chain), format!("replayed {outcome:?}, chain says {chain}")));
        }
    }
    let supply = if s["status"] == "Closed" { s["closed_supply"].as_str() } else { s["token_supply"].as_str() };
    l.check(
        "the progress tokens are exactly the confirmed gains",
        ensure(supply.and_then(|v| v.parse::<u64>().ok()) == Some(minted), format!("minted {minted}, chain supply {supply:?}")),
    );

    // Reveals after the season.
    println!("\n5. Tasks never opened (unused, or spent by a failed run), revealed after the season");
    let mut revealed_tasks = 0;
    for (i, r) in src.paged("get_reveals", count("reveals"), 50)?.iter().enumerate() {
        let Some(signed) = r["state"].get("Done") else { continue };
        let res = (|| {
            let (payload, digest) = verify_signed(&season, signed)?;
            let Payload::Reveal(p) = payload else { return Err("not a reveal".to_string()) };
            let ev = src.view("get_evidence", json!({ "key": format!("r{i}") }))?;
            let ev = ev.as_str().ok_or("its evidence was pruned")?.to_string();
            ensure(sha256_hex(ev.as_bytes()) == digest, "the stored evidence is not the signed evidence")?;
            let ev: Value = serde_json::from_str(&ev).map_err(|e| e.to_string())?;
            let asked: Vec<usize> = r["tasks"].as_array().cloned().unwrap_or_default().iter().map(|v| v.as_u64().unwrap_or(0) as usize).collect();
            let on_chain: Vec<String> = asked.iter().map(|&i| hex::encode(season.leaves[i])).collect();
            ensure(p.leaves == on_chain, "the signed leaves are not the tasks asked for")?;
            let revealed = ev["tasks"].as_array().cloned().unwrap_or_default();
            ensure(revealed.len() == asked.len(), "not every asked task is revealed")?;
            for (j, t) in revealed.iter().enumerate() {
                let task = Task::from_json(t.as_str().unwrap_or_default().as_bytes())?;
                ensure(hex::encode(task.leaf()) == p.leaves[j], "a revealed task does not hash to its leaf")?;
                let proof = merkle::proof(&season.leaves, asked[j]);
                ensure(merkle::verify(&task.leaf(), asked[j], season.leaves.len(), &proof, &season.root), "a revealed task does not prove into the root")?;
                task.validate()?;
                revealed_tasks += 1;
            }
            Ok(())
        })();
        l.check(format!("reveal {i}"), res);
    }
    println!("  {revealed_tasks} tasks opened and proven into the root");
    // Intel's collateral, saved with the snapshot: a browser cannot fetch it
    // from Intel's PCS (no CORS), and it is Intel-signed, so where it comes
    // from does not matter.
    if let (Some(i), Source::Rpc { export: Some(dir), .. }) = (&intel, src) {
        let out = json!({ "fetched_at": now, "by_platform": i.exported });
        std::fs::write(dir.join("intel-collateral.json"), serde_json::to_string(&out).unwrap()).map_err(|e| e.to_string())?;
    }
    Ok(l)
}
