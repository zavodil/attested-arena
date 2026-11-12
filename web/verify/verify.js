// Re-checks the season in the browser.
//
// State is read live from NEAR testnet (falling back to the snapshot saved
// next to this page when the RPC cannot be reached); every bulky thing that
// comes from this site — evidence blobs, Intel's collateral — is checked
// against a digest or a signature NEAR holds, so the site cannot change what
// it proves. The checks themselves are the judge's own Rust code
// (`verify/arena.wasm`: checkers, leaves, Merkle tree, request rebuild,
// ed25519, EIP-191) and Intel's quote verification (`dcap-qvl`, as WebAssembly).

import initDcap, { parse_quote, verify_quote } from './dcap-qvl/verifier.js';

const RPCS = ['https://rpc.testnet.fastnear.com', 'https://rpc.testnet.near.org'];
const ARENA = document.documentElement.dataset.arena;
const enc = new TextEncoder();
const dec = new TextDecoder();

let wasm;
async function loadArena() {
  if (wasm) return;
  const { instance } = await WebAssembly.instantiateStreaming(fetch('verify/arena.wasm'), {});
  wasm = instance.exports;
}
function core(req) {
  const b = enc.encode(JSON.stringify(req));
  const p = wasm.alloc(b.length);
  new Uint8Array(wasm.memory.buffer, p, b.length).set(b);
  const r = wasm.call(p, b.length);
  const rp = Number(r >> 32n), rl = Number(r & 0xffffffffn);
  const out = JSON.parse(dec.decode(new Uint8Array(wasm.memory.buffer, rp, rl)));
  wasm.dealloc(rp, rl);
  return out;
}

let source = 'NEAR testnet RPC';
async function rpc(method, params) {
  for (const url of RPCS) {
    try {
      const r = await fetch(url, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ jsonrpc: '2.0', id: 'v', method, params }) });
      const j = await r.json();
      if (j.result) return j.result;
    } catch (_) { /* the next endpoint */ }
  }
  throw new Error('no RPC answered');
}
async function hex16(text) {
  const d = await crypto.subtle.digest('SHA-256', enc.encode(text));
  return [...new Uint8Array(d)].map(b => b.toString(16).padStart(2, '0')).join('').slice(0, 16);
}
async function view(method, args = {}) {
  const a = JSON.stringify(args);
  try {
    const r = await rpc('query', { request_type: 'call_function', finality: 'final', account_id: ARENA, method_name: method, args_base64: btoa(a) });
    return JSON.parse(dec.decode(new Uint8Array(r.result)));
  } catch (_) {
    source = 'the snapshot saved with this page (the RPC did not answer)';
    const file = a === '{}' ? `${method}.json` : `${method}-${await hex16(a)}.json`;
    return (await fetch(`evidence/snapshot/${file}`)).json();
  }
}
async function txValue(hash, sender) {
  const r = await rpc('EXPERIMENTAL_tx_status', { tx_hash: hash, sender_account_id: sender, wait_until: 'FINAL' });
  const v = r.status && r.status.SuccessValue;
  return { value: v ? JSON.parse(dec.decode(Uint8Array.from(atob(v), c => c.charCodeAt(0)))) : null, raw: r };
}
const bytes = (h) => Uint8Array.from(h.match(/../g).map(x => parseInt(x, 16)));

// ---- the checks ---------------------------------------------------------------

const CLAIMS = {};
function claim(id) {
  CLAIMS[id] ??= { pass: 0, fail: [], notes: [], items: [] };
  return CLAIMS[id];
}
function check(id, ok, what) {
  const c = claim(id);
  if (ok) c.pass += 1; else c.fail.push(what);
  c.items.push((ok ? '✓ ' : '✗ ') + what);
}

async function run() {
  await loadArena();
  await initDcap({ module_or_path: 'verify/dcap-qvl/attestation-verifier.wasm' });
  const meta = await (await fetch('evidence/state.json')).json();
  const log = await (await fetch('evidence/transactions.json')).json();
  const experiments = await (await fetch('evidence/experiments.json')).json();
  const collateral = await (await fetch('evidence/snapshot/intel-collateral.json')).json();

  const s = await view('get_season');
  const leaves = await view('get_leaves', { from: 0, limit: 200 });
  const batches = await view('get_batches', { from: 0, limit: 50 });
  const portions = await view('get_portions', { from: 0, limit: 50 });
  const duels = await view('get_duels', { from: 0, limit: 50 });
  const reveals = await view('get_reveals', { from: 0, limit: 50 });
  const key = s.judge.key;
  const signedOk = (signed) => core({ op: 'judge', key, payload: signed.payload, signature: signed.signature }).ok;
  const named = (p) => p.arena === ARENA && p.operator === s.owner;

  // 1. The exam was fixed before the first duel, and every task used is one of it.
  check('exam', core({ op: 'root', leaves }).root === s.root, 'the Merkle root over the on-chain leaves is the root fixed at seal');
  claim('exam').notes.push(`${leaves.length} leaves, root ${s.root.slice(0, 16)}…`);

  // 2. Only this build wrote, sealed and scored it.
  const id = await txValue(meta.identity_tx, s.owner);
  const said = id.value && JSON.parse(id.value.payload).judge_key;
  check('judge', said === key && core({ op: 'judge', key, payload: id.value.payload, signature: id.value.signature }).ok,
    'the build said this key itself, signed with it');
  for (const [i, b] of batches.entries()) {
    if (!b.state.Done) continue;
    const p = JSON.parse(b.state.Done.payload);
    check('judge', signedOk(b.state.Done) && named(p), `batch ${i} is signed by the pinned build for this arena`);
    check('exam', JSON.stringify(p.leaves) === JSON.stringify(leaves.slice(b.first_leaf, b.first_leaf + p.leaves.length)), `batch ${i}'s leaves are on chain`);
  }

  // Portions: tasks, answers, attestation, scores.
  let answers = 0, quotes = 0;
  for (const [i, rec] of portions.entries()) {
    if (!rec.state.Done) continue;
    const signed = rec.state.Done;
    const p = JSON.parse(signed.payload);
    check('judge', signedOk(signed) && named(p), `portion ${i} is signed by the pinned build for this arena`);
    const evText = await view('get_evidence', { key: `p${i}` });
    check('judge', core({ op: 'sha256', text: evText }).sha256 === signed.evidence_sha256, `portion ${i}'s evidence is the evidence it signed`);
    const ev = JSON.parse(evText);
    ev.tasks.forEach((t, j) => {
      const leaf = core({ op: 'leaf', task: t });
      check('exam', leaf.leaf === leaves[rec.first_task + j], `task ${rec.first_task + j} hashes to its committed leaf`);
    });
    const models = [p.champion, p.challenger];
    const proven = [];
    for (const [k, model] of models.entries()) {
      const hashes = ev.tasks.map(t => core({ op: 'request', model, task: t }).sha256);
      const nonce = core({ op: 'nonce', hashes }).nonce;
      const att = ev.attestations[k];
      check('models', att.nonce === nonce, `portion ${i}: ${model} was attested for exactly these requests`);
      const ok = [];
      for (const inst of att.instances) {
        const q = bytes(inst.intel_quote);
        const fm = parse_quote(q).fmspc.toLowerCase();
        const col = Object.entries(collateral.by_platform).find(([k2]) => k2.toLowerCase().startsWith(fm));
        const v = col ? verify_quote(q, JSON.stringify(col[1]), collateral.fetched_at) : { ok: false };
        const suffix = parse_quote(q).report_data_suffix;
        const prefix = inst.signing_address.slice(2).toLowerCase().padEnd(64, '0');
        const claims = JSON.parse(atob(inst.gpu.token.split('.')[1].replace(/-/g, '+').replace(/_/g, '/')));
        const good = v.ok && v.report_data_prefix === prefix && suffix === nonce && claims['x-nvidia-overall-att-result'] === true;
        check('models', good, `portion ${i}: ${model}'s key ${inst.signing_address.slice(0, 10)}… — Intel quote ${v.status || 'not verified'}, bound to this run, GPU verdict`);
        quotes += 1;
        if (good) ok.push(inst.signing_address.toLowerCase());
      }
      proven.push(ok);
    }
    ev.answers.forEach((a, n) => {
      const j = Math.floor(n / 2), k = n % 2, model = models[k];
      const req = core({ op: 'request', model, task: ev.tasks[j] }).sha256;
      const text = `${model}:${req}:${core({ op: 'sha256', text: a.response }).sha256}`;
      const signer = a.signature.text === text ? core({ op: 'eip191', text, signature: a.signature.signature }).address : null;
      check('models', a.signature.signature_kind === 'provider_tee' && signer && proven[k].includes(signer),
        `portion ${i}, task ${rec.first_task + j}: ${model}'s answer is signed in its TEE over these exact bytes`);
      const g = core({ op: 'grade', task: ev.tasks[j], response: a.response });
      check('scores', g.ok === (k === 0 ? p.results[j].champion : p.results[j].challenger), `task ${rec.first_task + j}: the checker agrees with the signed score`);
      answers += 1;
    });
  }
  for (const [i, r] of reveals.entries()) {
    if (!r.state.Done) continue;
    check('judge', signedOk(r.state.Done), `reveal ${i} is signed by the pinned build`);
    const ev = JSON.parse(await view('get_evidence', { key: `r${i}` }));
    ev.tasks.forEach((t, j) => check('exam', core({ op: 'leaf', task: t }).leaf === leaves[r.tasks[j]], `task ${r.tasks[j]}, never opened, hashes to its leaf`));
  }
  claim('models').notes.push(`${answers} answers, ${quotes} Intel quotes verified in this browser`);
  claim('scores').notes.push(`${answers} answers re-scored with the judge's checker code`);

  // 5. Duels, tokens and payouts follow the rules.
  const sp = s.config.sprt, maxp = s.config.max_portions;
  let minted = 0;
  duels.forEach((d, i) => {
    let confirm = false, llr = 0, inPhase = 0, ct = 0, cc = 0, cx = 0, outcome = null;
    for (const id2 of d.portions) {
      const rec = portions[id2];
      if (!rec.state.Done || outcome) continue;
      for (const [c, x] of rec.results) {
        llr += (!c && x) ? sp.win : (c && !x) ? sp.loss : 0;
        if (confirm) { ct++; cc += c; cx += x; }
      }
      inPhase++;
      const acc = llr >= sp.accept, rej = llr <= sp.reject, out = inPhase >= maxp;
      if (!confirm) { if (acc) { confirm = true; llr = 0; inPhase = 0; } else if (rej) outcome = 'Rejected'; else if (out) outcome = 'NotProven'; }
      else if (acc) { const gain = Math.floor(Math.max(cx - cc, 0) * 10000 / Math.max(ct, 1)); minted += gain; outcome = JSON.stringify({ Record: { gain_centi: gain } }); }
      else if (rej || out) outcome = 'NotConfirmed';
    }
    const chain = typeof d.outcome === 'string' ? d.outcome : JSON.stringify(d.outcome);
    if (!outcome && chain === 'NotProven' && s.pool_tasks - s.next_task < s.config.portion_size) outcome = 'NotProven';
    check('money', outcome === chain, `duel ${i}: the test, replayed from the signed results, ends ${chain}`);
  });
  check('money', String(minted) === s.closed_supply, `progress tokens minted = the confirmed gains (${minted / 100})`);
  for (const t of log.filter(t => t.method === 'redeem' && t.status === 'ok')) {
    const r = (await txValue(t.hash, t.signer)).raw;
    const burn = r.receipts_outcome.flatMap(o => o.outcome.logs).find(l => l.includes('"ft_burn"'));
    const burned = burn ? Number(JSON.parse(burn.slice(11)).data[0].amount) : 0;
    const paid = r.receipts.filter(x => x.predecessor_id === ARENA).flatMap(x => (x.receipt.Action || { actions: [] }).actions).map(a => a.Transfer && a.Transfer.deposit).find(Boolean);
    const expected = (BigInt(burned) * BigInt(s.closed_pool_yocto) / BigInt(s.closed_supply)).toString();
    check('money', paid === expected, `${t.signer} redeemed ${burned / 100} tokens and was paid ${Number(BigInt(paid || 0) / 10n ** 21n) / 1000} NEAR = balance × pool / supply`);
  }

  // 6. Nobody could peek.
  const direct = (await txValue(experiments.direct.tx, s.owner)).value;
  check('peek', direct && /only from the arena/.test(direct.error || ''), 'the operator calling the judge directly for a sealed task: refused');
  const other = (await txValue(experiments.other_build.tx, s.owner)).value;
  check('peek', other && other.opened === false, 'another build declaring the judge’s exact key: the task does not open');
  const keys = await rpc('query', { request_type: 'view_access_key_list', finality: 'final', account_id: ARENA });
  check('peek', keys.keys.length === 0, `${ARENA} has no access key: its code cannot change`);
}

// ---- the page ---------------------------------------------------------------

let running = null;
function render() {
  for (const card of document.querySelectorAll('[data-claim]')) {
    const c = CLAIMS[card.dataset.claim];
    const out = card.querySelector('.result');
    if (!c) continue;
    const ok = c.fail.length === 0;
    card.classList.toggle('passed', ok);
    card.classList.toggle('failed', !ok);
    out.innerHTML = `<b>${ok ? '✓' : '✗'} ${c.pass} check${c.pass === 1 ? '' : 's'} passed${ok ? '' : `, ${c.fail.length} failed`}</b>` +
      (c.notes.length ? `<div class=muted>${c.notes.join(' · ')}</div>` : '') +
      (c.fail.length ? `<ul>${c.fail.map(f => `<li>✗ ${f}</li>`).join('')}</ul>` : '') +
      `<details class=items><summary>Every check (${c.items.length})</summary><ol>${c.items.map(t => `<li>${t}</li>`).join('')}</ol></details>`;
  }
  const total = Object.values(CLAIMS).reduce((n, c) => n + c.pass, 0);
  const failed = Object.values(CLAIMS).reduce((n, c) => n + c.fail.length, 0);
  document.querySelectorAll('.verify-summary').forEach(el => {
    el.textContent = `${total} checks passed, ${failed} failed — read from ${source}, checked in this browser.`;
  });
}
async function start(btn) {
  document.querySelectorAll('.recheck').forEach(b => { b.disabled = true; b.textContent = 'Checking…'; });
  try {
    running ??= run();
    await running;
    render();
    document.querySelectorAll('.recheck').forEach(b => { b.textContent = 'Checked ✓'; });
  } catch (e) {
    document.querySelectorAll('.verify-summary').forEach(el => { el.textContent = 'The check could not run: ' + e.message; });
    document.querySelectorAll('.recheck').forEach(b => { b.disabled = false; b.textContent = 'Re-check'; });
    running = null;
  }
}
document.querySelectorAll('.recheck').forEach(b => b.addEventListener('click', () => start(b)));
window.arenaVerify = { run: () => start(), claims: CLAIMS };
