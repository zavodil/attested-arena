//! Attested Arena — one season, on NEAR.
//!
//! The contract holds what everyone must be able to check and nobody can
//! change once the season runs:
//!
//! * **the sealed pool**: the leaf hash and ciphertext of every task, as the
//!   judge signed them, and the Merkle root over the leaves, fixed by `seal`
//!   before any duel;
//! * **the judge**: the URL and sha256 of its build, and the public key bound
//!   to that build. Every result is accepted only with that key's signature;
//! * **the duels**: which tasks each portion used, every result the judge
//!   signed (lost duels included), and the evidence behind it;
//! * **the money**: the sponsors' pool, the entrants' deposits, and the
//!   progress token (NEP-141) minted for confirmed records.
//!
//! The judge is reached only through this contract (`request_execution` on
//! OutLayer with the judge's wasm URL), and it acts only when the contract's
//! account has no full-access key. So this code, as published, decides which
//! tasks a portion opens, and the season's rules cannot be changed mid-season.
//!
//! Money out follows one rule per path: `redeem` pays `balance × pool /
//! supply`; with no record, `reclaim` returns each sponsor's own deposit;
//! `refund_deposit` returns each entry's deposit. Nothing else pays out.

mod merkle;
mod token;

#[cfg(test)]
mod tests;

use arena_core::payload::{self as wire, JudgeOutput, Payload};
use near_contract_standards::fungible_token::events::{FtBurn, FtMint};
use near_contract_standards::fungible_token::FungibleToken;
use near_sdk::json_types::U128;
use near_sdk::serde_json::{self, json};
use near_sdk::store::{LookupMap, Vector};
use near_sdk::{env, near, require, AccountId, BorshStorageKey, Gas, GasWeight, NearToken, PanicOnDefault, Promise, PromiseResult};

/// Kept for the callback that verifies and records a judge's result.
const CALLBACK_GAS: Gas = Gas::from_tgas(120);
/// The least `request_execution` is given; the rest of the transaction's gas
/// is added to it by weight. Attach 300 Tgas.
const EXECUTION_GAS: Gas = Gas::from_tgas(100);
/// The most tasks a portion or a reveal opens at once.
const MAX_OPEN: u32 = 20;

#[derive(BorshStorageKey)]
#[near]
enum Key {
    Leaves,
    Ciphertexts,
    Batches,
    Entries,
    Duels,
    Portions,
    Reveals,
    Opened,
    Evidence,
    Sponsors,
    Token,
}

#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    /// Batches are generated; no duel yet.
    Building,
    /// The pool is sealed: entries duel.
    Open,
    /// No more duels: tokens redeem, deposits return, unused tasks may be revealed.
    Closed,
}

#[near(serializers = [borsh, json])]
#[derive(Clone, Debug)]
pub struct Judge {
    pub wasm_url: String,
    /// Hex.
    pub wasm_sha256: String,
    /// The ed25519 key OutLayer's keystore derives for this build, hex.
    pub key: String,
}

/// The duel's sequential test, as integers (log ratios × 10^6).
#[near(serializers = [borsh, json])]
#[derive(Clone, Copy, Debug)]
pub struct Sprt {
    pub win: i64,
    pub loss: i64,
    pub accept: i64,
    pub reject: i64,
}

#[near(serializers = [borsh, json])]
#[derive(Clone, Debug)]
pub struct Config {
    /// The model that writes tasks, and the one that reads `seating` stories back.
    pub generator: String,
    pub reader: String,
    /// The first champion.
    pub base_model: String,
    /// The models an entry may name.
    pub models: Vec<String>,
    pub portion_size: u32,
    /// Portions per phase before a duel ends undecided.
    pub max_portions: u32,
    pub sprt: Sprt,
    /// What an entry attaches; returned after the season.
    pub entry_deposit: U128,
    /// The operator's secret row holding the NEAR AI key.
    pub secrets_profile: String,
    pub max_execution_seconds: u32,
    pub max_instructions: u64,
}

#[near(serializers = [borsh, json])]
#[derive(Clone, Debug)]
pub struct Entry {
    pub account: AccountId,
    pub model: String,
    pub deposit: U128,
    pub refunded: bool,
}

#[near(serializers = [borsh, json])]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Test,
    Confirm,
}

#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DuelOutcome {
    /// Confirmed better; `gain_centi` is the gain in hundredths of a point,
    /// measured on the confirmation tasks only, and the tokens minted.
    Record { gain_centi: u32 },
    /// The test showed it is not better.
    Rejected,
    /// The test ran out of portions undecided.
    NotProven,
    /// It passed the test but not the confirmation.
    NotConfirmed,
}

#[near(serializers = [borsh, json])]
#[derive(Clone, Debug)]
pub struct Duel {
    pub entry: u32,
    pub challenger: AccountId,
    pub challenger_model: String,
    pub champion_model: String,
    pub champion_holder: Option<AccountId>,
    pub phase: Phase,
    pub llr: i64,
    pub portions_in_phase: u32,
    pub portions: Vec<u32>,
    pub confirm_tasks: u32,
    pub confirm_champion: u32,
    pub confirm_challenger: u32,
    pub outcome: Option<DuelOutcome>,
}

/// A judge result as received: the payload text and signature, kept so
/// anyone can verify them against the judge's key from chain state alone.
#[near(serializers = [borsh, json])]
#[derive(Clone, Debug)]
pub struct Signed {
    pub payload: String,
    pub signature: String,
    pub evidence_sha256: String,
}

#[near(serializers = [borsh, json])]
#[derive(Clone, Debug)]
pub enum CallState {
    Pending,
    Done(Signed),
    /// The run produced no valid result; its reason, as far as it is known.
    Failed(String),
}

#[near(serializers = [borsh, json])]
#[derive(Clone, Debug)]
pub struct BatchRecord {
    pub family: String,
    pub first_leaf: u32,
    pub admitted: u32,
    pub generated: u32,
    pub rejected: Vec<(String, u32)>,
    pub state: CallState,
}

#[near(serializers = [borsh, json])]
#[derive(Clone, Debug)]
pub struct PortionRecord {
    pub duel: u32,
    pub portion: u32,
    pub phase: Phase,
    pub first_task: u32,
    pub count: u32,
    /// Per task: (champion right, challenger right).
    pub results: Vec<(bool, bool)>,
    pub state: CallState,
}

#[near(serializers = [borsh, json])]
#[derive(Clone, Debug)]
pub struct RevealRecord {
    pub tasks: Vec<u32>,
    pub state: CallState,
}

#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Call {
    Generate { batch: u32 },
    Portion { record: u32 },
    Reveal { record: u32 },
}

#[near(contract_state)]
#[derive(PanicOnDefault)]
pub struct Arena {
    owner: AccountId,
    outlayer: AccountId,
    judge: Judge,
    config: Config,
    status: Status,
    leaves: Vector<[u8; 32]>,
    ciphertexts: LookupMap<u32, Vec<u8>>,
    root: Option<[u8; 32]>,
    next_task: u32,
    batches: Vector<BatchRecord>,
    entries: Vector<Entry>,
    next_entry: u32,
    duels: Vector<Duel>,
    active: Option<u32>,
    champion_model: String,
    champion_holder: Option<AccountId>,
    portions: Vector<PortionRecord>,
    reveals: Vector<RevealRecord>,
    /// Task index → how it was opened (a portion's id, or a reveal's id + 2^31).
    opened: LookupMap<u32, u32>,
    /// Evidence by call: `b{n}`, `p{n}`, `r{n}`. Prunable after the season.
    evidence: LookupMap<String, String>,
    pending: Option<Call>,
    pool: u128,
    sponsors: LookupMap<AccountId, u128>,
    closed_pool: u128,
    closed_supply: u128,
    token: FungibleToken,
}

#[near]
impl Arena {
    /// Initialise in the deploy transaction itself, then remove the account's
    /// full-access key: the judge acts only for a locked arena.
    #[init]
    pub fn new(owner: AccountId, outlayer: AccountId, judge: Judge, config: Config) -> Self {
        require!(judge.key.len() == 64 && hex_32(&judge.key).is_some(), "the judge's key is 32 bytes of hex");
        require!(judge.wasm_sha256.len() == 64 && hex_32(&judge.wasm_sha256).is_some(), "the judge's sha256 is 32 bytes of hex");
        require!(judge.wasm_url.starts_with("https://"), "the judge's URL is https");
        require!(config.portion_size >= 1 && config.portion_size <= MAX_OPEN, "a portion has 1 to 20 tasks");
        require!(config.max_portions >= 1, "a phase has at least one portion");
        require!(config.sprt.win > 0 && config.sprt.loss < 0 && config.sprt.accept > 0 && config.sprt.reject < 0, "the test's steps and bounds have their signs");
        require!(config.max_execution_seconds >= 1 && config.max_execution_seconds <= 180, "a run lasts 1 to 180 seconds");
        require!(!config.base_model.is_empty(), "a base model is named");
        Self {
            champion_model: config.base_model.clone(),
            champion_holder: None,
            owner,
            outlayer,
            judge,
            config,
            status: Status::Building,
            leaves: Vector::new(Key::Leaves),
            ciphertexts: LookupMap::new(Key::Ciphertexts),
            root: None,
            next_task: 0,
            batches: Vector::new(Key::Batches),
            entries: Vector::new(Key::Entries),
            next_entry: 0,
            duels: Vector::new(Key::Duels),
            active: None,
            portions: Vector::new(Key::Portions),
            reveals: Vector::new(Key::Reveals),
            opened: LookupMap::new(Key::Opened),
            evidence: LookupMap::new(Key::Evidence),
            pending: None,
            pool: 0,
            sponsors: LookupMap::new(Key::Sponsors),
            closed_pool: 0,
            closed_supply: 0,
            token: FungibleToken::new(Key::Token),
        }
    }

    // ---- the pool -------------------------------------------------------

    /// Ask the judge for a batch of tasks of `family`. Attach NEAR for the run
    /// (`estimate_execution_cost` on OutLayer); what it does not use returns
    /// to the owner.
    #[payable]
    pub fn generate(&mut self, family: String, count: u32) -> Promise {
        self.assert_owner();
        require!(self.status == Status::Building, "the pool is sealed");
        require!(self.pending.is_none(), "a judge run is in flight");
        let batch = self.batches.len();
        self.batches.push(BatchRecord {
            family: family.clone(),
            first_leaf: self.leaves.len(),
            admitted: 0,
            generated: 0,
            rejected: vec![],
            state: CallState::Pending,
        });
        self.pending = Some(Call::Generate { batch });
        let input = json!({
            "op": "generate", "arena": env::current_account_id(), "batch": batch, "family": family, "count": count,
            "generator": self.config.generator, "reader": self.config.reader,
        });
        self.call_judge(input).then(Self::ext(env::current_account_id()).with_static_gas(CALLBACK_GAS).with_unused_gas_weight(0).on_generated(batch))
    }

    #[private]
    pub fn on_generated(&mut self, batch: u32) {
        self.pending = None;
        let mut record = self.batches[batch].clone();
        match self.accept_batch(batch, &record, promise_output()) {
            Ok((signed, b, evidence)) => {
                record.admitted = b.leaves.len() as u32;
                record.generated = b.generated;
                record.rejected = b.rejected.into_iter().collect();
                record.state = CallState::Done(signed);
                self.evidence.insert(format!("b{batch}"), evidence);
            }
            Err(why) => {
                env::log_str(&format!("batch {batch} failed: {why}"));
                record.state = CallState::Failed(why);
            }
        }
        self.batches[batch] = record;
    }

    /// Fix the pool: the Merkle root over every leaf, before any duel.
    pub fn seal(&mut self) {
        self.assert_owner();
        require!(self.status == Status::Building, "the pool is already sealed");
        require!(self.pending.is_none(), "a judge run is in flight");
        require!(!self.leaves.is_empty(), "the pool is empty");
        let leaves: Vec<[u8; 32]> = self.leaves.iter().copied().collect();
        self.root = merkle::root(&leaves);
        self.status = Status::Open;
        env::log_str(&format!("sealed: {} tasks, root {}", leaves.len(), to_hex(&self.root.unwrap())));
    }

    // ---- entries and duels -------------------------------------------

    /// Put a model forward. Attach exactly the entry deposit; it returns after
    /// the season, whatever happens.
    #[payable]
    pub fn enter(&mut self, model: String) -> u32 {
        require!(self.status != Status::Closed, "the season is closed");
        require!(self.config.models.contains(&model), "this model is not one the season admits");
        require!(env::attached_deposit().as_yoctonear() == self.config.entry_deposit.0, "attach exactly the entry deposit");
        self.entries.push(Entry { account: env::predecessor_account_id(), model, deposit: self.config.entry_deposit, refunded: false });
        self.entries.len() - 1
    }

    /// The next entry challenges the champion.
    pub fn start_duel(&mut self) -> u32 {
        self.assert_owner();
        require!(self.status == Status::Open, "the pool is not sealed, or the season is closed");
        require!(self.active.is_none(), "a duel is running");
        require!(self.next_entry < self.entries.len(), "no entry is waiting");
        let entry = self.entries[self.next_entry].clone();
        let duel = Duel {
            entry: self.next_entry,
            challenger: entry.account,
            challenger_model: entry.model,
            champion_model: self.champion_model.clone(),
            champion_holder: self.champion_holder.clone(),
            phase: Phase::Test,
            llr: 0,
            portions_in_phase: 0,
            portions: vec![],
            confirm_tasks: 0,
            confirm_champion: 0,
            confirm_challenger: 0,
            outcome: None,
        };
        self.next_entry += 1;
        self.duels.push(duel);
        self.active = Some(self.duels.len() - 1);
        self.duels.len() - 1
    }

    /// The next portion of the running duel: the next unused tasks of the pool,
    /// in order, answered by both models.
    #[payable]
    pub fn run_portion(&mut self) -> Promise {
        self.assert_owner();
        require!(self.pending.is_none(), "a judge run is in flight");
        let id = self.active.unwrap_or_else(|| env::panic_str("no duel is running"));
        let duel = self.duels[id].clone();
        let count = self.config.portion_size;
        require!(self.next_task + count <= self.leaves.len(), "the pool has too few unused tasks left");
        let record = self.portions.len();
        let first = self.next_task;
        self.next_task += count;
        let portion = duel.portions.len() as u32;
        self.portions.push(PortionRecord { duel: id, portion, phase: duel.phase, first_task: first, count, results: vec![], state: CallState::Pending });
        let mut d = duel.clone();
        d.portions.push(record);
        self.duels[id] = d;
        self.pending = Some(Call::Portion { record });
        let input = json!({
            "op": "portion", "arena": env::current_account_id(), "duel": id, "portion": portion,
            "phase": match duel.phase { Phase::Test => "test", Phase::Confirm => "confirm" },
            "champion": duel.champion_model, "challenger": duel.challenger_model,
            "tasks": self.sealed(first..first + count),
        });
        self.call_judge(input).then(Self::ext(env::current_account_id()).with_static_gas(CALLBACK_GAS).with_unused_gas_weight(0).on_portion(record))
    }

    #[private]
    pub fn on_portion(&mut self, record: u32) {
        self.pending = None;
        let mut rec = self.portions[record].clone();
        match self.accept_portion(&rec, promise_output()) {
            Ok((signed, results, evidence)) => {
                rec.results = results.iter().map(|o| (o.champion, o.challenger)).collect();
                rec.state = CallState::Done(signed);
                self.evidence.insert(format!("p{record}"), evidence);
                for i in rec.first_task..rec.first_task + rec.count {
                    self.opened.insert(i, record);
                }
                self.portions[record] = rec.clone();
                self.apply(rec.duel, &rec.results);
            }
            Err(why) => {
                // The tasks may have reached the models: they are spent, and
                // the duel goes on with fresh ones.
                env::log_str(&format!("portion {record} failed, its tasks are spent: {why}"));
                rec.state = CallState::Failed(why);
                self.portions[record] = rec;
            }
        }
    }

    /// The running duel cannot get another portion: fewer unused tasks are
    /// left than a portion takes. It ends undecided — no record, nothing
    /// minted — so the season can close.
    pub fn end_duel(&mut self) {
        self.assert_owner();
        require!(self.pending.is_none(), "a judge run is in flight");
        let id = self.active.unwrap_or_else(|| env::panic_str("no duel is running"));
        require!(self.next_task + self.config.portion_size > self.leaves.len(), "the pool can still fill a portion");
        let mut d = self.duels[id].clone();
        d.outcome = Some(DuelOutcome::NotProven);
        self.duels[id] = d;
        self.active = None;
    }

    // ---- the season's end ----------------------------------------------

    /// No more duels. The pool and the token supply are fixed for payouts.
    pub fn close(&mut self) {
        self.assert_owner();
        require!(self.status == Status::Open, "the season is not running");
        require!(self.active.is_none() && self.pending.is_none(), "a duel or a judge run is in flight");
        self.status = Status::Closed;
        self.closed_pool = self.pool;
        self.closed_supply = self.token.total_supply;
    }

    /// Open tasks no duel used, so the whole pool can be audited.
    #[payable]
    pub fn reveal(&mut self, tasks: Vec<u32>) -> Promise {
        self.assert_owner();
        require!(self.status == Status::Closed, "tasks are revealed after the season");
        require!(self.pending.is_none(), "a judge run is in flight");
        require!(!tasks.is_empty() && tasks.len() as u32 <= MAX_OPEN, "reveal 1 to 20 tasks at a time");
        for &i in &tasks {
            require!(i < self.leaves.len(), "no such task");
            require!(!self.opened.contains_key(&i), "this task is already open");
        }
        let record = self.reveals.len();
        self.reveals.push(RevealRecord { tasks: tasks.clone(), state: CallState::Pending });
        self.pending = Some(Call::Reveal { record });
        let sealed: Vec<_> = tasks.iter().map(|&i| self.sealed_one(i)).collect();
        let input = json!({ "op": "reveal", "arena": env::current_account_id(), "tasks": sealed });
        self.call_judge(input).then(Self::ext(env::current_account_id()).with_static_gas(CALLBACK_GAS).with_unused_gas_weight(0).on_revealed(record))
    }

    #[private]
    pub fn on_revealed(&mut self, record: u32) {
        self.pending = None;
        let mut rec = self.reveals[record].clone();
        match self.accept_reveal(&rec, promise_output()) {
            Ok((signed, evidence)) => {
                rec.state = CallState::Done(signed);
                self.evidence.insert(format!("r{record}"), evidence);
                for &i in &rec.tasks {
                    self.opened.insert(i, (1 << 31) | record);
                }
            }
            Err(why) => {
                env::log_str(&format!("reveal {record} failed: {why}"));
                rec.state = CallState::Failed(why);
            }
        }
        self.reveals[record] = rec;
    }

    /// After the season, drop stored evidence (its sha256 stays in the signed
    /// payload) and free the storage.
    pub fn prune(&mut self, key: String) {
        self.assert_owner();
        require!(self.status == Status::Closed, "evidence is kept while the season runs");
        self.evidence.remove(&key);
    }

    // ---- money -----------------------------------------------------------

    /// Add to the season's pool.
    #[payable]
    pub fn sponsor(&mut self) {
        require!(self.status != Status::Closed, "the season is closed");
        let amount = env::attached_deposit().as_yoctonear();
        require!(amount > 0, "attach NEAR to sponsor");
        let who = env::predecessor_account_id();
        let before = self.sponsors.get(&who).copied().unwrap_or(0);
        self.sponsors.insert(who, before + amount);
        self.pool += amount;
    }

    /// Exchange all of the caller's progress tokens for their share of the
    /// pool: `balance × pool / supply`, both fixed at close.
    pub fn redeem(&mut self) -> Promise {
        require!(self.status == Status::Closed, "tokens redeem after the season");
        require!(self.closed_supply > 0, "no record was set; sponsors reclaim");
        let who = env::predecessor_account_id();
        let balance = self.token.accounts.get(&who).unwrap_or(0);
        require!(balance > 0, "no tokens to redeem");
        let amount = balance * self.closed_pool / self.closed_supply;
        self.token.internal_withdraw(&who, balance);
        FtBurn { owner_id: &who, amount: U128(balance), memo: Some("redeemed") }.emit();
        Promise::new(who).transfer(NearToken::from_yoctonear(amount))
    }

    /// With no record in the season, each sponsor takes back their deposit.
    pub fn reclaim(&mut self) -> Promise {
        require!(self.status == Status::Closed, "the season is not closed");
        require!(self.closed_supply == 0, "a record was set: the pool goes to the token holders");
        let who = env::predecessor_account_id();
        let amount = self.sponsors.remove(&who).unwrap_or_else(|| env::panic_str("nothing to reclaim"));
        Promise::new(who).transfer(NearToken::from_yoctonear(amount))
    }

    /// An entry's deposit returns to it after the season, whatever its duel did.
    pub fn refund_deposit(&mut self, entry: u32) -> Promise {
        require!(self.status == Status::Closed, "deposits return after the season");
        let mut e = self.entries.get(entry).cloned().unwrap_or_else(|| env::panic_str("no such entry"));
        require!(e.account == env::predecessor_account_id(), "only the entry's account takes its deposit");
        require!(!e.refunded, "already refunded");
        e.refunded = true;
        let amount = e.deposit.0;
        let account = e.account.clone();
        self.entries[entry] = e;
        Promise::new(account).transfer(NearToken::from_yoctonear(amount))
    }

    // ---- views -------------------------------------------------------------

    pub fn get_season(&self) -> serde_json::Value {
        json!({
            "arena": env::current_account_id(),
            "status": self.status,
            "owner": self.owner,
            "outlayer": self.outlayer,
            "judge": self.judge,
            "config": self.config,
            "pool_tasks": self.leaves.len(),
            "root": self.root.map(|r| to_hex(&r)),
            "next_task": self.next_task,
            "champion": { "model": self.champion_model, "holder": self.champion_holder },
            "active_duel": self.active,
            "pending": self.pending,
            "entries": self.entries.len(),
            "duels": self.duels.len(),
            "portions": self.portions.len(),
            "batches": self.batches.len(),
            "reveals": self.reveals.len(),
            "pool_yocto": U128(self.pool),
            "token_supply": U128(self.token.total_supply),
            "closed_pool_yocto": U128(self.closed_pool),
            "closed_supply": U128(self.closed_supply),
        })
    }

    pub fn get_leaves(&self, from: u32, limit: u32) -> Vec<String> {
        (from..(from + limit).min(self.leaves.len())).map(|i| to_hex(&self.leaves[i])).collect()
    }

    /// Base64.
    pub fn get_ciphertext(&self, index: u32) -> Option<String> {
        self.ciphertexts.get(&index).map(|c| near_sdk::base64::Engine::encode(&near_sdk::base64::engine::general_purpose::STANDARD, c))
    }

    pub fn get_batches(&self, from: u32, limit: u32) -> Vec<BatchRecord> {
        (from..(from + limit).min(self.batches.len())).map(|i| self.batches[i].clone()).collect()
    }

    pub fn get_entries(&self, from: u32, limit: u32) -> Vec<Entry> {
        (from..(from + limit).min(self.entries.len())).map(|i| self.entries[i].clone()).collect()
    }

    pub fn get_duels(&self, from: u32, limit: u32) -> Vec<Duel> {
        (from..(from + limit).min(self.duels.len())).map(|i| self.duels[i].clone()).collect()
    }

    pub fn get_portions(&self, from: u32, limit: u32) -> Vec<PortionRecord> {
        (from..(from + limit).min(self.portions.len())).map(|i| self.portions[i].clone()).collect()
    }

    pub fn get_reveals(&self, from: u32, limit: u32) -> Vec<RevealRecord> {
        (from..(from + limit).min(self.reveals.len())).map(|i| self.reveals[i].clone()).collect()
    }

    pub fn get_evidence(&self, key: String) -> Option<String> {
        self.evidence.get(&key).cloned()
    }

    pub fn get_sponsor(&self, account: AccountId) -> U128 {
        U128(self.sponsors.get(&account).copied().unwrap_or(0))
    }
}

// ---- internals ---------------------------------------------------------------

impl Arena {
    fn assert_owner(&self) {
        require!(env::predecessor_account_id() == self.owner, "only the operator runs the season");
    }

    /// `request_execution` on OutLayer: the judge's build, directly by URL;
    /// the operator's NEAR AI key row; the whole deposit, its unused part
    /// returned to the operator.
    fn call_judge(&self, input: serde_json::Value) -> Promise {
        let args = json!({
            "source": { "WasmUrl": { "url": self.judge.wasm_url, "hash": self.judge.wasm_sha256, "build_target": "wasm32-wasip2" } },
            "input_data": input.to_string(),
            "resource_limits": {
                "max_instructions": self.config.max_instructions,
                "max_memory_mb": 1024,
                "max_execution_seconds": self.config.max_execution_seconds,
            },
            "secrets_ref": { "profile": self.config.secrets_profile, "account_id": self.owner },
            "response_format": "Json",
            "payer_account_id": self.owner,
        });
        Promise::new(self.outlayer.clone()).function_call_weight(
            "request_execution".to_string(),
            args.to_string().into_bytes(),
            env::attached_deposit(),
            EXECUTION_GAS,
            GasWeight(1),
        )
    }

    fn sealed_one(&self, index: u32) -> serde_json::Value {
        let ct = self.ciphertexts.get(&index).unwrap_or_else(|| env::panic_str("a task has no ciphertext"));
        json!({
            "leaf": to_hex(&self.leaves[index]),
            "ciphertext": near_sdk::base64::Engine::encode(&near_sdk::base64::engine::general_purpose::STANDARD, ct),
        })
    }

    fn sealed(&self, range: std::ops::Range<u32>) -> Vec<serde_json::Value> {
        range.map(|i| self.sealed_one(i)).collect()
    }

    /// Verify a judge's output: the build's key signed exactly this payload,
    /// the payload names this arena and its operator, and the evidence hashes
    /// to what the payload says.
    fn verify(&self, output: Result<Vec<u8>, String>) -> Result<(Signed, Payload, String), String> {
        let bytes = output?;
        let value: Option<serde_json::Value> = serde_json::from_slice(&bytes).map_err(|e| format!("the run's output is not JSON: {e}"))?;
        let value = value.ok_or("the run produced no output")?;
        if let Some(error) = value.get("error").and_then(|e| e.as_str()) {
            return Err(format!("the judge refused: {error}"));
        }
        let out: JudgeOutput = serde_json::from_value(value).map_err(|e| format!("the run's output is not a judge result: {e}"))?;
        let signature: [u8; 64] = hex_bytes(&out.signature).and_then(|b| b.try_into().ok()).ok_or("the signature is not 64 bytes of hex")?;
        let key = hex_32(&self.judge.key).ok_or("the pinned key is not hex")?;
        if !env::ed25519_verify(&signature, &wire::signing_message(&out.payload), &key) {
            return Err("the result is not signed by this arena's judge build".to_string());
        }
        let payload: Payload = serde_json::from_str(&out.payload).map_err(|e| format!("the payload does not read: {e}"))?;
        let (arena, operator, evidence_sha256) = match &payload {
            Payload::Batch(b) => (&b.arena, &b.operator, &b.evidence_sha256),
            Payload::Portion(p) => (&p.arena, &p.operator, &p.evidence_sha256),
            Payload::Reveal(r) => (&r.arena, &r.operator, &r.evidence_sha256),
            Payload::Identity(_) => return Err("an identity is not a result".to_string()),
        };
        if arena != env::current_account_id().as_str() {
            return Err(format!("the result is for the arena {arena}"));
        }
        if operator != self.owner.as_str() {
            return Err(format!("the result was produced for the operator {operator}"));
        }
        if to_hex(&env::sha256_array(out.evidence.as_bytes())) != *evidence_sha256 {
            return Err("the evidence is not the evidence the payload names".to_string());
        }
        let signed = Signed { payload: out.payload, signature: out.signature, evidence_sha256: evidence_sha256.clone() };
        Ok((signed, payload, out.evidence))
    }

    fn accept_batch(&mut self, batch: u32, record: &BatchRecord, output: Result<Vec<u8>, String>) -> Result<(Signed, wire::Batch, String), String> {
        let (signed, payload, evidence) = self.verify(output)?;
        let Payload::Batch(b) = payload else { return Err("not a batch".to_string()) };
        if b.batch != batch || b.family != record.family {
            return Err("the result is for another batch".to_string());
        }
        if b.leaves.len() != b.ciphertexts.len() {
            return Err("leaves and ciphertexts do not pair".to_string());
        }
        #[derive(near_sdk::serde::Deserialize)]
        #[serde(crate = "near_sdk::serde")]
        struct Cts {
            ciphertexts: Vec<String>,
        }
        let cts: Cts = serde_json::from_str(&evidence).map_err(|e| format!("the batch's evidence does not read: {e}"))?;
        if cts.ciphertexts.len() != b.leaves.len() {
            return Err("the evidence holds another number of ciphertexts".to_string());
        }
        let mut decoded = Vec::new();
        for (i, ct) in cts.ciphertexts.iter().enumerate() {
            let bytes = near_sdk::base64::Engine::decode(&near_sdk::base64::engine::general_purpose::STANDARD, ct).map_err(|_| "a ciphertext is not base64".to_string())?;
            if to_hex(&env::sha256_array(&bytes)) != b.ciphertexts[i] {
                return Err("a ciphertext is not the one the payload names".to_string());
            }
            let leaf = hex_32(&b.leaves[i]).ok_or("a leaf is not 32 bytes of hex")?;
            decoded.push((leaf, bytes));
        }
        for (leaf, bytes) in decoded {
            let index = self.leaves.len();
            self.leaves.push(leaf);
            self.ciphertexts.insert(index, bytes);
        }
        Ok((signed, b, evidence))
    }

    fn accept_portion(&self, rec: &PortionRecord, output: Result<Vec<u8>, String>) -> Result<(Signed, Vec<wire::Outcome>, String), String> {
        let (signed, payload, evidence) = self.verify(output)?;
        let Payload::Portion(p) = payload else { return Err("not a portion".to_string()) };
        let duel = &self.duels[rec.duel];
        let phase = match rec.phase {
            Phase::Test => wire::Phase::Test,
            Phase::Confirm => wire::Phase::Confirm,
        };
        if p.duel != rec.duel || p.portion != rec.portion || p.phase != phase {
            return Err("the result is for another portion".to_string());
        }
        if p.champion != duel.champion_model || p.challenger != duel.challenger_model {
            return Err("the result is for other models".to_string());
        }
        let assigned: Vec<String> = (rec.first_task..rec.first_task + rec.count).map(|i| to_hex(&self.leaves[i])).collect();
        if p.leaves != assigned {
            return Err("the result is for other tasks than this portion was given".to_string());
        }
        if p.results.len() != assigned.len() {
            return Err("the result does not score every task".to_string());
        }
        Ok((signed, p.results, evidence))
    }

    fn accept_reveal(&self, rec: &RevealRecord, output: Result<Vec<u8>, String>) -> Result<(Signed, String), String> {
        let (signed, payload, evidence) = self.verify(output)?;
        let Payload::Reveal(r) = payload else { return Err("not a reveal".to_string()) };
        let asked: Vec<String> = rec.tasks.iter().map(|&i| to_hex(&self.leaves[i])).collect();
        if r.leaves != asked {
            return Err("the result reveals other tasks than were asked".to_string());
        }
        Ok((signed, evidence))
    }

    /// Run a judged portion's results through the duel's test.
    fn apply(&mut self, id: u32, results: &[(bool, bool)]) {
        let sprt = self.config.sprt;
        let mut d = self.duels[id].clone();
        for &(champion, challenger) in results {
            d.llr += match (champion, challenger) {
                (false, true) => sprt.win,
                (true, false) => sprt.loss,
                _ => 0,
            };
            if d.phase == Phase::Confirm {
                d.confirm_tasks += 1;
                d.confirm_champion += champion as u32;
                d.confirm_challenger += challenger as u32;
            }
        }
        d.portions_in_phase += 1;
        let accept = d.llr >= sprt.accept;
        let reject = d.llr <= sprt.reject;
        let out_of_portions = d.portions_in_phase >= self.config.max_portions;
        match d.phase {
            Phase::Test if accept => {
                d.phase = Phase::Confirm;
                d.llr = 0;
                d.portions_in_phase = 0;
            }
            Phase::Test if reject => d.outcome = Some(DuelOutcome::Rejected),
            Phase::Test if out_of_portions => d.outcome = Some(DuelOutcome::NotProven),
            Phase::Confirm if accept => {
                // The gain, in hundredths of a point, on the confirmation tasks.
                let diff = d.confirm_challenger.saturating_sub(d.confirm_champion);
                let gain_centi = diff * 10_000 / d.confirm_tasks.max(1);
                if gain_centi > 0 {
                    self.mint(&d.challenger, gain_centi as u128);
                }
                self.champion_model = d.challenger_model.clone();
                self.champion_holder = Some(d.challenger.clone());
                d.outcome = Some(DuelOutcome::Record { gain_centi });
            }
            Phase::Confirm if reject || out_of_portions => d.outcome = Some(DuelOutcome::NotConfirmed),
            _ => {}
        }
        if d.outcome.is_some() {
            self.active = None;
        }
        self.duels[id] = d;
    }

    fn mint(&mut self, account: &AccountId, amount: u128) {
        if !self.token.accounts.contains_key(account) {
            self.token.internal_register_account(account);
        }
        self.token.internal_deposit(account, amount);
        FtMint { owner_id: account, amount: U128(amount), memo: Some("confirmed record") }.emit();
    }
}

/// The result `request_execution` returned, or why there is none.
fn promise_output() -> Result<Vec<u8>, String> {
    match env::promise_result(0) {
        PromiseResult::Successful(bytes) => Ok(bytes),
        PromiseResult::Failed => Err("request_execution failed (its receipt carries the reason)".to_string()),
    }
}

pub(crate) fn to_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(HEX[(b >> 4) as usize] as char);
        s.push(HEX[(b & 15) as usize] as char);
    }
    s
}

pub(crate) fn hex_bytes(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 {
        return None;
    }
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok()).collect()
}

pub(crate) fn hex_32(s: &str) -> Option<[u8; 32]> {
    hex_bytes(s)?.try_into().ok()
}
