//! Each test is one claim the arena makes, checked against the contract.

use super::*;
use near_contract_standards::fungible_token::core::FungibleTokenCore;
use arena_core::payload::{Batch as WireBatch, Outcome, Payload, Phase as WirePhase, Portion as WirePortion};
use arena_core::sha256_hex;
use ed25519_dalek::{Signer, SigningKey};
use near_sdk::test_utils::VMContextBuilder;
use near_sdk::{testing_env, PromiseResult};
use std::collections::BTreeMap;

const OWNER: &str = "operator.testnet";
const ARENA: &str = "season1.arena.testnet";

fn judge_key() -> SigningKey {
    SigningKey::from_bytes(&[7u8; 32])
}

fn other_build_key() -> SigningKey {
    SigningKey::from_bytes(&[8u8; 32])
}

fn context(predecessor: &str, deposit: u128) -> VMContextBuilder {
    let mut c = VMContextBuilder::new();
    c.current_account_id(ARENA.parse().unwrap())
        .predecessor_account_id(predecessor.parse().unwrap())
        .signer_account_id(predecessor.parse().unwrap())
        .attached_deposit(NearToken::from_yoctonear(deposit))
        .account_balance(NearToken::from_near(100));
    c
}

fn set(predecessor: &str, deposit: u128) {
    testing_env!(context(predecessor, deposit).build());
}

/// Deliver a judge run's output to the callback that is waiting for it.
fn deliver(output: &serde_json::Value) {
    testing_env!(
        context(ARENA, 0).build(),
        near_sdk::test_vm_config(),
        near_sdk::RuntimeFeesConfig::test(),
        Default::default(),
        vec![PromiseResult::Successful(output.to_string().into_bytes())]
    );
}

fn config() -> Config {
    let s = arena_core::sprt::Sprt::arena();
    Config {
        generator: "gen/model".into(),
        reader: "reader/model".into(),
        base_model: "base/model".into(),
        models: vec!["strong/model".into(), "base/model".into()],
        portion_size: 10,
        max_portions: 3,
        sprt: Sprt { win: s.win, loss: s.loss, accept: s.accept, reject: s.reject },
        entry_deposit: U128(1_000),
        secrets_profile: "arena".into(),
        max_execution_seconds: 180,
        max_instructions: 100_000_000_000,
    }
}

fn arena() -> Arena {
    set(OWNER, 0);
    Arena::new(
        OWNER.parse().unwrap(),
        "outlayer.testnet".parse().unwrap(),
        Judge {
            wasm_url: "https://main.fastfs.io/judge.wasm".into(),
            wasm_sha256: "ab".repeat(32),
            key: hex_of(&judge_key().verifying_key().to_bytes()),
        },
        config(),
    )
}

fn hex_of(b: &[u8]) -> String {
    to_hex(b)
}

fn signed_output(key: &SigningKey, payload: &Payload, evidence: &serde_json::Value) -> serde_json::Value {
    let payload = serde_json::to_string(payload).unwrap();
    let signature = key.sign(&arena_core::payload::signing_message(&payload));
    json!({ "payload": payload, "signature": hex_of(&signature.to_bytes()), "evidence": evidence.to_string() })
}

/// A batch of `n` tasks, as the judge would sign it.
fn batch_output(key: &SigningKey, batch: u32, n: usize, arena: &str, operator: &str) -> serde_json::Value {
    let cts: Vec<Vec<u8>> = (0..n).map(|i| format!("ciphertext {batch}/{i}").into_bytes()).collect();
    let evidence = json!({ "ciphertexts": cts.iter().map(|c| near_sdk::base64::Engine::encode(&near_sdk::base64::engine::general_purpose::STANDARD, c)).collect::<Vec<_>>() });
    let payload = Payload::Batch(WireBatch {
        arena: arena.into(),
        operator: operator.into(),
        batch,
        family: "seating".into(),
        leaves: (0..n).map(|i| sha256_hex(format!("task {batch}/{i}").as_bytes())).collect(),
        ciphertexts: cts.iter().map(|c| sha256_hex(c)).collect(),
        generated: n as u32 + 2,
        rejected: BTreeMap::from([("statement_mismatch".to_string(), 2)]),
        evidence_sha256: sha256_hex(evidence.to_string().as_bytes()),
    });
    signed_output(key, &payload, &evidence)
}

/// A sealed arena with `tasks` tasks and one entry waiting.
fn open_arena(tasks: usize) -> Arena {
    let mut a = arena();
    set(OWNER, 1);
    let _ = a.generate("seating".into(), tasks as u32);
    deliver(&batch_output(&judge_key(), 0, tasks, ARENA, OWNER));
    a.on_generated(0);
    set(OWNER, 0);
    a.seal();
    set("alice.testnet", 1_000);
    a.enter("strong/model".into());
    set(OWNER, 0);
    a.start_duel();
    a
}

/// Run one portion and deliver the judge's verdicts for it.
fn portion(a: &mut Arena, results: &[(bool, bool)]) -> u32 {
    set(OWNER, 1);
    let _ = a.run_portion();
    let record = a.portions.len() - 1;
    let rec = a.portions[record].clone();
    let duel = a.duels[rec.duel].clone();
    let evidence = json!({ "tasks": [] });
    let payload = Payload::Portion(WirePortion {
        arena: ARENA.into(),
        operator: OWNER.into(),
        duel: rec.duel,
        portion: rec.portion,
        phase: if rec.phase == Phase::Test { WirePhase::Test } else { WirePhase::Confirm },
        champion: duel.champion_model,
        challenger: duel.challenger_model,
        leaves: (rec.first_task..rec.first_task + rec.count).map(|i| to_hex(&a.leaves[i])).collect(),
        results: results.iter().map(|&(c, x)| Outcome { champion: c, challenger: x }).collect(),
        evidence_sha256: sha256_hex(evidence.to_string().as_bytes()),
    });
    deliver(&signed_output(&judge_key(), &payload, &evidence));
    a.on_portion(record);
    record
}

#[test]
fn the_on_chain_root_is_the_same_tree_as_the_judges() {
    for n in 1..=9 {
        let leaves: Vec<[u8; 32]> = (0..n).map(|i| arena_core::merkle::leaf_hash(format!("t{i}").as_bytes())).collect();
        set(OWNER, 0);
        assert_eq!(merkle::root(&leaves), arena_core::merkle::root(&leaves), "n={n}");
    }
}

#[test]
fn a_batch_signed_by_the_pinned_build_enters_the_pool() {
    let mut a = arena();
    set(OWNER, 1);
    let _ = a.generate("seating".into(), 6);
    assert_eq!(a.pending, Some(Call::Generate { batch: 0 }));
    deliver(&batch_output(&judge_key(), 0, 6, ARENA, OWNER));
    a.on_generated(0);
    assert_eq!(a.leaves.len(), 6);
    assert!(matches!(a.batches[0].state, CallState::Done(_)));
    assert_eq!(a.batches[0].rejected, vec![("statement_mismatch".to_string(), 2)], "the admission counts are public");
    assert!(a.get_evidence("b0".into()).is_some());
    assert!(a.pending.is_none());
}

#[test]
fn a_result_signed_by_any_other_build_is_refused() {
    let mut a = arena();
    set(OWNER, 1);
    let _ = a.generate("seating".into(), 6);
    deliver(&batch_output(&other_build_key(), 0, 6, ARENA, OWNER));
    a.on_generated(0);
    assert_eq!(a.leaves.len(), 0);
    match &a.batches[0].state {
        CallState::Failed(why) => assert!(why.contains("not signed by this arena's judge build"), "{why}"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_result_made_for_another_arena_or_operator_is_refused() {
    for (arena_id, operator, expected) in [("other.testnet", OWNER, "for the arena"), (ARENA, "mallory.testnet", "for the operator")] {
        let mut a = arena();
        set(OWNER, 1);
        let _ = a.generate("seating".into(), 3);
        deliver(&batch_output(&judge_key(), 0, 3, arena_id, operator));
        a.on_generated(0);
        assert_eq!(a.leaves.len(), 0);
        assert!(matches!(&a.batches[0].state, CallState::Failed(why) if why.contains(expected)));
    }
}

#[test]
fn evidence_other_than_the_signed_evidence_is_refused() {
    let mut a = arena();
    set(OWNER, 1);
    let _ = a.generate("seating".into(), 3);
    let mut out = batch_output(&judge_key(), 0, 3, ARENA, OWNER);
    out["evidence"] = json!(json!({ "ciphertexts": ["c3dhcHBlZA=="] }).to_string());
    deliver(&out);
    a.on_generated(0);
    assert!(matches!(&a.batches[0].state, CallState::Failed(why) if why.contains("not the evidence the payload names")));
}

#[test]
fn the_judges_refusal_is_recorded_and_nothing_enters_the_pool() {
    let mut a = arena();
    set(OWNER, 1);
    let _ = a.generate("seating".into(), 3);
    deliver(&json!({ "error": "this operation is taken only from the arena season1.arena.testnet itself" }));
    a.on_generated(0);
    assert!(matches!(&a.batches[0].state, CallState::Failed(why) if why.contains("judge refused")));
}

#[test]
fn nothing_duels_before_the_pool_is_sealed_and_nothing_is_added_after() {
    let mut a = arena();
    set("alice.testnet", 1_000);
    a.enter("strong/model".into());
    set(OWNER, 0);
    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| a.start_duel())).is_err());
    let mut a = open_arena(30);
    set(OWNER, 1);
    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = a.generate("seating".into(), 3);
    }))
    .is_err());
    assert_eq!(a.get_season()["root"].as_str().unwrap().len(), 64);
}

#[test]
fn a_portion_must_score_exactly_the_tasks_it_was_given() {
    let mut a = open_arena(30);
    set(OWNER, 1);
    let _ = a.run_portion();
    let duel = a.duels[0].clone();
    let evidence = json!({});
    let payload = Payload::Portion(WirePortion {
        arena: ARENA.into(),
        operator: OWNER.into(),
        duel: 0,
        portion: 0,
        phase: WirePhase::Test,
        champion: duel.champion_model,
        challenger: duel.challenger_model,
        // Tasks 10..20 instead of 0..10: the judge was not given these.
        leaves: (10..20).map(|i| to_hex(&a.leaves[i])).collect(),
        results: vec![Outcome { champion: false, challenger: true }; 10],
        evidence_sha256: sha256_hex(evidence.to_string().as_bytes()),
    });
    deliver(&signed_output(&judge_key(), &payload, &evidence));
    a.on_portion(0);
    assert!(matches!(&a.portions[0].state, CallState::Failed(why) if why.contains("other tasks")));
    assert_eq!(a.duels[0].llr, 0, "a refused result moves nothing");
}

#[test]
fn a_failed_run_spends_its_tasks_and_the_duel_goes_on() {
    let mut a = open_arena(30);
    set(OWNER, 1);
    let _ = a.run_portion();
    testing_env!(
        context(ARENA, 0).build(),
        near_sdk::test_vm_config(),
        near_sdk::RuntimeFeesConfig::test(),
        Default::default(),
        vec![PromiseResult::Successful(b"null".to_vec())]
    );
    a.on_portion(0);
    assert!(matches!(a.portions[0].state, CallState::Failed(_)));
    assert_eq!(a.next_task, 10, "the tasks that may have reached the models are never used again");
    portion(&mut a, &[(true, true); 10]);
    assert_eq!(a.portions[1].first_task, 10);
}

#[test]
fn a_better_model_is_confirmed_and_paid_for_the_confirmation_gain_only() {
    let mut a = open_arena(60);
    // Test phase: the challenger wins 9 tasks the champion misses; 1 tie.
    let mut test = vec![(false, true); 9];
    test.push((true, true));
    portion(&mut a, &test);
    assert_eq!(a.duels[0].phase, Phase::Confirm, "nine discordant wins pass the test");
    // Confirmation, on fresh tasks: wins 9, loses 0, both right on 1. The gain
    // is measured here: challenger 10/10, champion 1/10 → +90 points.
    portion(&mut a, &test);
    assert_eq!(a.duels[0].outcome, Some(DuelOutcome::Record { gain_centi: 9_000 }));
    assert_eq!(a.ft_balance_of("alice.testnet".parse().unwrap()).0, 9_000);
    assert_eq!(a.get_season()["champion"]["model"], "strong/model");
    assert!(a.active.is_none());
}

#[test]
fn entering_the_champions_own_model_earns_nothing() {
    let mut a = open_arena(60);
    // The same model answers the same: no discordant task, the test never moves.
    for _ in 0..3 {
        portion(&mut a, &[(true, true), (false, false), (true, true), (true, true), (false, false), (true, true), (true, true), (false, false), (true, true), (true, true)]);
    }
    assert_eq!(a.duels[0].outcome, Some(DuelOutcome::NotProven));
    assert_eq!(a.token.total_supply, 0);
    assert_eq!(a.get_season()["champion"]["model"], "base/model");
}

#[test]
fn a_challenger_that_passes_by_luck_fails_the_confirmation() {
    let mut a = open_arena(60);
    let mut lucky = vec![(false, true); 9];
    lucky.push((false, false));
    portion(&mut a, &lucky);
    assert_eq!(a.duels[0].phase, Phase::Confirm);
    let mut real = vec![(true, false); 5];
    real.extend(vec![(true, true); 5]);
    portion(&mut a, &real);
    assert_eq!(a.duels[0].outcome, Some(DuelOutcome::NotConfirmed));
    assert_eq!(a.token.total_supply, 0);
}

#[test]
fn money_out_is_one_rule_per_path() {
    let mut a = open_arena(60);
    set("sponsor.testnet", 9_000_000);
    a.sponsor();
    let mut test = vec![(false, true); 9];
    test.push((true, true));
    portion(&mut a, &test);
    portion(&mut a, &test);
    set(OWNER, 0);
    a.close();
    assert_eq!((a.closed_pool, a.closed_supply), (9_000_000, 9_000));
    // A third of the tokens moves to bob; each redeems balance × pool / supply.
    set("alice.testnet", 1);
    a.token.internal_register_account(&"bob.testnet".parse().unwrap());
    a.token.internal_transfer(&"alice.testnet".parse().unwrap(), &"bob.testnet".parse().unwrap(), 3_000, None);
    set("bob.testnet", 0);
    let _ = a.redeem();
    assert_eq!(a.token.total_supply, 6_000, "redeemed tokens are burned");
    assert_eq!(a.closed_supply, 9_000, "the rate is fixed at close: later redeemers get the same");
    set("alice.testnet", 0);
    let _ = a.redeem();
    assert_eq!(a.token.total_supply, 0);
    set("sponsor.testnet", 0);
    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = a.reclaim();
    }))
    .is_err(), "a record was set: the sponsor's money is the token holders'");
    set("alice.testnet", 0);
    let _ = a.refund_deposit(0);
    assert!(a.entries[0].refunded);
}

#[test]
fn with_no_record_each_sponsor_takes_back_their_own() {
    let mut a = open_arena(60);
    set("sponsor.testnet", 5_000);
    a.sponsor();
    portion(&mut a, &[(true, false); 10]);
    assert_eq!(a.duels[0].outcome, Some(DuelOutcome::Rejected));
    set(OWNER, 0);
    a.close();
    set("sponsor.testnet", 0);
    let _ = a.reclaim();
    assert_eq!(a.get_sponsor("sponsor.testnet".parse().unwrap()).0, 0);
    set("stranger.testnet", 0);
    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = a.reclaim();
    }))
    .is_err());
}

#[test]
fn only_the_operator_runs_the_season() {
    let mut a = arena();
    set("mallory.testnet", 1);
    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = a.generate("seating".into(), 3);
    }))
    .is_err());
    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| a.seal())).is_err());
}

#[test]
fn a_duel_the_pool_cannot_finish_ends_undecided() {
    let mut a = open_arena(15);
    set(OWNER, 0);
    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| a.end_duel())).is_err(), "the pool can still fill a portion");
    portion(&mut a, &[(false, true); 10]);
    set(OWNER, 0);
    a.end_duel();
    assert_eq!(a.duels[0].outcome, Some(DuelOutcome::NotProven));
    assert_eq!(a.token.total_supply, 0);
    a.close();
}
