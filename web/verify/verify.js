// Re-checks the season in the browser.
//
// The page opens by checking the copy of the arena's NEAR state saved next to
// it (`evidence/snapshot/`, with the block it was read at); one button reads
// the same answers live from NEAR testnet and checks again. Every bulky thing
// that comes from this site — evidence blobs, Intel's collateral — is checked
// against a digest or a signature NEAR holds. The checks themselves are the
// judge's own Rust code (`verify/arena.wasm`: checkers, leaves, Merkle tree,
// request rebuild, ed25519, EIP-191) and Intel's quote verification
// (`dcap-qvl`, as WebAssembly).

import initDcap, { parse_quote, verify_quote } from './dcap-qvl/verifier.js';

const RPCS = ['https://rpc.testnet.fastnear.com', 'https://rpc.testnet.near.org'];
const EXPLORER = 'https://testnet.nearblocks.io';
const ARENA = document.documentElement.dataset.arena;
const enc = new TextEncoder();
const dec = new TextDecoder();

let wasm;
async function loadCode() {
  if (wasm) return;
  const { instance } = await WebAssembly.instantiateStreaming(fetch('verify/arena.wasm'), {});
  await initDcap({ module_or_path: 'verify/dcap-qvl/attestation-verifier.wasm' });
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

// ---- reading NEAR: live, or the copy saved with the page -------------------------

const json = async (path) => { const r = await fetch(path); if (!r.ok) throw new Error(`${path}: HTTP ${r.status}`); return r.json(); };
let slots = 6;
const waiting = [];
async function limited(fn) {
  while (slots === 0) await new Promise(r => waiting.push(r));
  slots -= 1;
  try { return await fn(); } finally { slots += 1; waiting.shift()?.(); }
}
async function rpc(method, params) {
  return limited(async () => {
    for (const url of RPCS) {
      try {
        const r = await fetch(url, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ jsonrpc: '2.0', id: 'v', method, params }) });
        const j = await r.json();
        if (j.result) return j.result;
      } catch (_) { /* the next endpoint */ }
    }
    throw new Error(`NEAR RPC did not answer ${method}`);
  });
}
async function hex16(text) {
  const d = await crypto.subtle.digest('SHA-256', enc.encode(text));
  return [...new Uint8Array(d)].map(b => b.toString(16).padStart(2, '0')).join('').slice(0, 16);
}
function reader(live) {
  return {
    async view(method, args = {}) {
      const a = JSON.stringify(args);
      if (!live) return json(`evidence/snapshot/${a === '{}' ? `${method}.json` : `${method}-${await hex16(a)}.json`}`);
      const r = await rpc('query', { request_type: 'call_function', finality: 'final', account_id: ARENA, method_name: method, args_base64: btoa(a) });
      return JSON.parse(dec.decode(new Uint8Array(r.result)));
    },
    async tx(hash, sender) {
      const r = live ? await rpc('EXPERIMENTAL_tx_status', { tx_hash: hash, sender_account_id: sender, wait_until: 'FINAL' })
        : await json(`evidence/snapshot/tx-${hash}.json`);
      const v = r.status && r.status.SuccessValue;
      return { value: v ? JSON.parse(dec.decode(Uint8Array.from(atob(v), c => c.charCodeAt(0)))) : null, raw: r };
    },
    keys() {
      return live ? rpc('query', { request_type: 'view_access_key_list', finality: 'final', account_id: ARENA }) : json('evidence/snapshot/access-keys.json');
    },
  };
}
const bytes = (h) => Uint8Array.from(h.match(/../g).map(x => parseInt(x, 16)));

// ---- what each group of checks means -------------------------------------------------

const GROUPS = {
  root: ['exam', 'The commitment', 'The Merkle tree is rebuilt here from every leaf the arena stores. Its root must equal the root the arena fixed at <code>seal</code>, before the first duel. Change one task, and its leaf and this root change.'],
  batch: ['exam', 'Batches', 'Each batch the judge wrote lists its leaves in the payload it signed. The arena must hold exactly those leaves, in order.'],
  task: ['exam', 'Tasks used in duels', 'Each task was published with its duel\'s result. Hashed here — sha256 of <code>0x00</code> and the task\'s canonical JSON — it must give the leaf committed before any duel: the task a model was scored on is the task that was sealed.'],
  unopened: ['exam', 'Tasks never opened', 'The tasks of a portion that failed before any model saw them, published after the season. They hash to their leaves too.'],
  identity: ['judge', 'The judge\'s key', 'The build announced its key in its own run on NEAR and signed the announcement with it. The arena pinned that key; it accepts nothing signed by another.'],
  signed: ['judge', 'Signatures', 'Every batch, portion and reveal the arena accepted carries an ed25519 signature by the pinned key, checked here, over a payload that names this arena and its operator.'],
  evidence: ['judge', 'Evidence', 'The bulky evidence of a portion — tasks, answers, attestations — is stored separately. Its sha256, computed here, must be the digest the judge signed.'],
  nonce: ['models', 'Requests', 'Each request a model received is rebuilt here, byte for byte, from the task with the judge\'s own code. The hash of those requests is the nonce NEAR AI attested for that model.'],
  quote: ['models', 'Hardware attestation', 'Each Intel TDX quote is verified here with Intel\'s quote verifier against Intel\'s collateral. Its report data must hold the model\'s signing key and the nonce of exactly these requests; NVIDIA\'s verdict on the GPUs must pass.'],
  answer: ['models', 'Signed answers', 'Each answer\'s EIP-191 signature covers the model, the request hash and the response hash. The address recovered here must be a key the quotes above proved lives in the TEE.'],
  grade: ['scores', 'Scores', 'Each answer is scored again here with the judge\'s checker code, compiled to WebAssembly. The verdict must equal the score the judge signed.'],
  duel: ['money', 'Duels', 'Each duel\'s sequential test is replayed here from the signed results, portion by portion. It must end the way the arena recorded.'],
  mint: ['money', 'Tokens', 'Progress tokens are minted only for a confirmed gain, measured on the confirmation tasks.'],
  redeem: ['money', 'Payouts', 'Each payout must be exactly balance × pool ÷ supply.'],
  peek: ['peek', 'Attempts to peek', 'Run on NEAR during the season, on purpose. Each must have failed.'],
};
const CLAIMS = {};
function claim(id) {
  CLAIMS[id] ??= { pass: 0, fail: 0, notes: [], groups: {} };
  return CLAIMS[id];
}
function check(group, ok, what, here = '', chain = '') {
  const c = claim(GROUPS[group][0]);
  if (ok) c.pass += 1; else c.fail += 1;
  (c.groups[group] ??= []).push({ ok, what, here, chain });
}
const esc = (t) => String(t).replace(/[&<>"']/g, ch => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[ch]));
const h = (x, n = 12) => `<code title="${esc(x)}">${esc(String(x).slice(0, n))}…</code>`;
const txa = (hash, label) => `<a href="${EXPLORER}/txns/${esc(hash)}" target=_blank rel=noopener>${esc(label || hash.slice(0, 8) + '…')}</a>`;
const near = (y) => `${Number(BigInt(y) / 10n ** 21n) / 1000} NEAR`;
const short = (m) => esc(m.split('/').pop());
const taskLine = (n, t) => `task ${n} · ${esc(t.family)} · <span class=muted title="${esc(t.statement)}">${esc(t.statement.slice(0, 70))}…</span>`;

// ---- the checks ---------------------------------------------------------------

async function run(live) {
  const t0 = performance.now();
  const r = reader(live);
  const [, meta, log, experiments, collateral, saved] = await Promise.all([loadCode(), json('evidence/state.json'),
    json('evidence/transactions.json'), json('evidence/experiments.json'), json('evidence/snapshot/intel-collateral.json'),
    json('evidence/snapshot/meta.json')]);
  const [s, leaves, batches, portions, duels, reveals] = await Promise.all([r.view('get_season'),
    r.view('get_leaves', { from: 0, limit: 200 }), r.view('get_batches', { from: 0, limit: 50 }), r.view('get_portions', { from: 0, limit: 50 }),
    r.view('get_duels', { from: 0, limit: 50 }), r.view('get_reveals', { from: 0, limit: 50 })]);
  const redeems = log.filter(t => t.method === 'redeem' && t.status === 'ok');
  const [portionEv, revealEv, id, redeemTx, direct, other, keys] = await Promise.all([
    Promise.all(portions.map((p, i) => p.state.Done ? r.view('get_evidence', { key: `p${i}` }) : null)),
    Promise.all(reveals.map((x, i) => x.state.Done ? r.view('get_evidence', { key: `r${i}` }) : null)),
    r.tx(meta.identity_tx, s.owner), Promise.all(redeems.map(t => r.tx(t.hash, t.signer))),
    r.tx(experiments.direct.tx, s.owner), r.tx(experiments.other_build.tx, s.owner), r.keys()]);
  for (const k of Object.keys(CLAIMS)) delete CLAIMS[k];
  const key = s.judge.key;
  const sigOk = (signed) => core({ op: 'judge', key, payload: signed.payload, signature: signed.signature }).ok;
  const named = (p) => p.arena === ARENA && p.operator === s.owner;
  const sealTx = log.find(t => t.method === 'seal' && t.status === 'ok');

  // The exam was fixed before the first duel, and every task used is one of it.
  const root = core({ op: 'root', leaves }).root;
  check('root', root === s.root, `the root over all ${leaves.length} leaves`, h(root, 16), `${h(s.root, 16)}, fixed by ${sealTx ? txa(sealTx.hash, 'seal') : 'seal'}`);
  claim('exam').notes.push(`${leaves.length} leaves, root ${s.root.slice(0, 16)}…`);

  // Only this build wrote, sealed and scored it.
  const said = id.value && JSON.parse(id.value.payload).judge_key;
  check('identity', said === key && core({ op: 'judge', key, payload: id.value.payload, signature: id.value.signature }).ok,
    'the build said this key itself, signed with it', `announced ${h(said || '—', 16)} in ${txa(meta.identity_tx, 'its run')}`, `pinned ${h(key, 16)}`);
  for (const [i, b] of batches.entries()) {
    if (!b.state.Done) continue;
    const p = JSON.parse(b.state.Done.payload);
    check('signed', sigOk(b.state.Done) && named(p), `batch ${i}`, 'ed25519 by the pinned key: valid', `names ${esc(p.arena)}`);
    const onChain = leaves.slice(b.first_leaf, b.first_leaf + p.leaves.length);
    check('batch', JSON.stringify(p.leaves) === JSON.stringify(onChain), `batch ${i} · ${esc(b.family)} · ${p.leaves.length} tasks`,
      `signed: ${h(p.leaves[0] || '', 8)} … ${h(p.leaves.at(-1) || '', 8)}`, `the same as leaves ${b.first_leaf}–${b.first_leaf + p.leaves.length - 1}`);
  }

  // Portions: tasks, answers, attestation, scores.
  let answers = 0, quotes = 0;
  for (const [i, rec] of portions.entries()) {
    if (!rec.state.Done) continue;
    const signed = rec.state.Done;
    const p = JSON.parse(signed.payload);
    check('signed', sigOk(signed) && named(p), `portion ${i}`, 'ed25519 by the pinned key: valid', `names ${esc(p.arena)}`);
    const evText = portionEv[i];
    const evSha = core({ op: 'sha256', text: evText }).sha256;
    check('evidence', evSha === signed.evidence_sha256, `portion ${i}'s evidence, ${Math.round(evText.length / 1024)} KB`, h(evSha, 16), `signed: ${h(signed.evidence_sha256, 16)}`);
    const ev = JSON.parse(evText);
    ev.tasks.forEach((t, j) => {
      const n = rec.first_task + j, leaf = core({ op: 'leaf', task: t }).leaf;
      check('task', leaf === leaves[n], taskLine(n, JSON.parse(t)), h(leaf), `leaf ${n}: ${h(leaves[n] || '—')}`);
    });
    const models = [p.champion, p.challenger];
    const proven = [];
    for (const [k, model] of models.entries()) {
      const hashes = ev.tasks.map(t => core({ op: 'request', model, task: t }).sha256);
      const nonce = core({ op: 'nonce', hashes }).nonce;
      const att = ev.attestations[k];
      check('nonce', att.nonce === nonce, `portion ${i} · ${short(model)} · ${hashes.length} requests`, `nonce ${h(nonce)}`, `attested: ${h(att.nonce)}`);
      const ok = [];
      for (const inst of att.instances) {
        const q = bytes(inst.intel_quote);
        const parsed = parse_quote(q);
        const fm = parsed.fmspc.toLowerCase();
        const col = Object.entries(collateral.by_platform).find(([k2]) => k2.toLowerCase().startsWith(fm));
        const v = col ? verify_quote(q, JSON.stringify(col[1]), collateral.fetched_at) : { ok: false };
        const prefix = inst.signing_address.slice(2).toLowerCase().padEnd(64, '0');
        const claims = JSON.parse(atob(inst.gpu.token.split('.')[1].replace(/-/g, '+').replace(/_/g, '/')));
        const gpu = claims['x-nvidia-overall-att-result'] === true;
        const good = v.ok && v.report_data_prefix === prefix && parsed.report_data_suffix === nonce && gpu;
        check('quote', good, `portion ${i} · ${short(model)} · key ${h(inst.signing_address, 10)}`,
          `Intel: ${esc(v.status || 'not verified')}; report data = this key ‖ ${h(parsed.report_data_suffix, 8)}`, `NVIDIA GPUs: ${gpu ? 'pass' : 'fail'}`);
        quotes += 1;
        if (good) ok.push(inst.signing_address.toLowerCase());
      }
      proven.push(ok);
    }
    ev.answers.forEach((a, n) => {
      const j = Math.floor(n / 2), k = n % 2, model = models[k], task = rec.first_task + j;
      const req = core({ op: 'request', model, task: ev.tasks[j] }).sha256;
      const text = `${model}:${req}:${core({ op: 'sha256', text: a.response }).sha256}`;
      const signer = a.signature.text === text ? core({ op: 'eip191', text, signature: a.signature.signature }).address : null;
      const attested = signer && proven[k].includes(signer);
      check('answer', a.signature.signature_kind === 'provider_tee' && attested,
        `task ${task} · ${short(model)}`, `signer recovered: ${h(signer || 'none', 10)}`, attested ? 'a key attested above' : 'not an attested key');
      const g = core({ op: 'grade', task: ev.tasks[j], response: a.response });
      const signedScore = k === 0 ? p.results[j].champion : p.results[j].challenger;
      check('grade', g.ok === signedScore, `task ${task} · ${short(model)}`, `checker: ${g.ok ? 'solved' : 'not solved'}`, `judge signed: ${signedScore ? 'solved' : 'not solved'}`);
      answers += 1;
    });
  }
  for (const [i, x] of reveals.entries()) {
    if (!x.state.Done) continue;
    check('signed', sigOk(x.state.Done), `reveal ${i}`, 'ed25519 by the pinned key: valid', '');
    JSON.parse(revealEv[i]).tasks.forEach((t, j) => {
      const n = x.tasks[j], leaf = core({ op: 'leaf', task: t }).leaf;
      check('unopened', leaf === leaves[n], taskLine(n, JSON.parse(t)), h(leaf), `leaf ${n}: ${h(leaves[n] || '—')}`);
    });
  }
  claim('models').notes.push(`${answers} answers, ${quotes} Intel quotes verified in this browser`);
  claim('scores').notes.push(`${answers} answers re-scored with the judge's checker code`);

  // Duels, tokens and payouts follow the rules.
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
    const show = (o) => o && o.startsWith('{') ? `record, +${JSON.parse(o).Record.gain_centi / 100} points` : esc(o || 'unfinished');
    check('duel', outcome === chain, `duel ${i} · ${d.portions.length} portions`, `replayed: ${show(outcome)}`, `arena: ${show(chain)}`);
  });
  check('mint', String(minted) === s.closed_supply, 'progress tokens minted', `confirmed gains: ${minted / 100}`, `token supply: ${Number(s.closed_supply) / 100}`);
  redeems.forEach((t, n) => {
    const raw = redeemTx[n].raw;
    const burn = raw.receipts_outcome.flatMap(o => o.outcome.logs).find(l => l.includes('"ft_burn"'));
    const burned = burn ? Number(JSON.parse(burn.slice(11)).data[0].amount) : 0;
    const paid = raw.receipts.filter(x => x.predecessor_id === ARENA).flatMap(x => (x.receipt.Action || { actions: [] }).actions).map(a => a.Transfer && a.Transfer.deposit).find(Boolean);
    const expected = (BigInt(burned) * BigInt(s.closed_pool_yocto) / BigInt(s.closed_supply)).toString();
    check('redeem', paid === expected, `${esc(t.signer)} redeemed ${burned / 100} tokens`,
      `${burned / 100} × ${near(s.closed_pool_yocto)} ÷ ${Number(s.closed_supply) / 100} = ${near(expected)}`, `paid ${near(paid || '0')} in ${txa(t.hash)}`);
  });

  // Nobody could peek.
  check('peek', direct.value && /only from the arena/.test(direct.value.error || ''), 'the operator calls the judge directly for a sealed task',
    `the judge answered: “${esc((direct.value && direct.value.error) || '—')}”`, txa(experiments.direct.tx));
  check('peek', other.value && other.value.opened === false, 'another build declares the judge\'s exact key and tries to open a task',
    `opened: ${esc(String(other.value && other.value.opened))}`, txa(experiments.other_build.tx));
  check('peek', keys.keys.length === 0, `${esc(ARENA)} could change its code`, `access keys: ${keys.keys.length}`,
    `<a href="${EXPLORER}/address/${esc(ARENA)}" target=_blank rel=noopener>the account</a>`);

  return { live, ms: Math.round(performance.now() - t0), saved };
}

// ---- the page ---------------------------------------------------------------

function groupHtml(g, rows) {
  const [, title, explain] = GROUPS[g];
  const bad = rows.filter(x => !x.ok).length;
  return `<section class=vgroup><h4>${title} <span class=${bad ? 'bad' : 'ok'}>${bad ? `✗ ${bad} of ${rows.length} failed` : `✓ ${rows.length} of ${rows.length}`}</span></h4><p>${explain}</p>` +
    `<table class=vt><tr><th></th><th>What</th><th>Computed in this browser</th><th>On NEAR</th></tr>` +
    rows.map(x => `<tr class=${x.ok ? 'vok' : 'vbad'}><td>${x.ok ? '✓' : '✗'}</td><td>${x.what}</td><td>${x.here}</td><td>${x.chain}</td></tr>`).join('') + '</table></section>';
}
function dialog(id, title, body) {
  let d = document.getElementById(id);
  if (!d) {
    d = document.createElement('dialog');
    d.id = id;
    d.className = 'vdialog';
    d.addEventListener('click', e => { if (e.target === d) d.close(); });
    document.body.append(d);
  }
  d.innerHTML = `<div class=vhead><h3>${title}</h3><button class=vclose aria-label=Close>×</button></div><div class=vbody>${body}</div>`;
  d.querySelector('.vclose').addEventListener('click', () => d.close());
  return d;
}
let last = null;
function render(res) {
  last = res;
  for (const card of document.querySelectorAll('[data-claim]')) {
    const c = CLAIMS[card.dataset.claim];
    const out = card.querySelector('.result');
    if (!c) continue;
    const ok = c.fail === 0;
    card.classList.toggle('passed', ok);
    card.classList.toggle('failed', !ok);
    out.innerHTML = `<b>${ok ? '✓' : '✗'} ${c.pass} check${c.pass === 1 ? '' : 's'} passed${ok ? '' : `, ${c.fail} failed`}</b>` +
      (c.notes.length ? `<div class=muted>${c.notes.join(' · ')}</div>` : '') +
      `<button class=vlink data-open="${card.dataset.claim}">See every check, with its values →</button>`;
  }
  const total = Object.values(CLAIMS).reduce((n, c) => n + c.pass, 0);
  const failed = Object.values(CLAIMS).reduce((n, c) => n + c.fail, 0);
  const from = res.live ? 'reading NEAR testnet live' : `on NEAR's state at block ${res.saved.block_height.toLocaleString('en')}, saved with this page`;
  document.querySelectorAll('.verify-summary').forEach(el => {
    el.innerHTML = `<b class=${failed ? 'bad' : 'ok'}>${total} checks passed, ${failed} failed</b> — run by this browser just now, in ${(res.ms / 1000).toFixed(1)} s, ${from}.`;
  });
}
function openClaim(id) {
  const card = document.querySelector(`[data-claim="${id}"]`);
  const c = CLAIMS[id];
  dialog('vclaim', esc(card.querySelector('h4').textContent),
    `<p class=muted>${last.live ? 'Read live from NEAR testnet.' : `NEAR's state at block ${last.saved.block_height}, saved ${esc(last.saved.saved_at)}.`} ` +
    `Every value in the middle column was computed by this page when it opened. Hover a shortened hash for the whole of it.</p>` +
    Object.entries(c.groups).map(([g, rows]) => groupHtml(g, rows)).join('')).showModal();
}
function openHow() {
  const sv = last ? last.saved : null;
  dialog('vhow', 'How this page checks the season',
    `<p>Nothing here is a recorded result. When the page opens, it runs every check itself with the judge's own Rust code — the checkers, ` +
    `leaves, Merkle tree, request rebuild, ed25519 and EIP-191 — compiled to WebAssembly (<a href="verify/arena.wasm">arena.wasm</a>, built ` +
    `from <code>verify-web/</code>), and with Intel's quote verifier <code>dcap-qvl</code>, also as WebAssembly.</p>` +
    `<h4>Where the data comes from</h4><ul>` +
    `<li><b>The arena's state</b> — season, leaves, batches, portions, duels, reveals and evidence — as NEAR returned it for ` +
    `<a href="${EXPLORER}/address/${esc(ARENA)}" target=_blank rel=noopener>${esc(ARENA)}</a>` +
    (sv ? ` at block <a href="${EXPLORER}/blocks/${esc(sv.block_hash)}" target=_blank rel=noopener>${sv.block_height}</a> (${esc(sv.saved_at)})` : '') +
    `, saved in <code>evidence/snapshot/</code> by <code>scripts/snapshot.py</code>. The season is closed: this state no longer changes.</li>` +
    `<li><b>Four transactions</b> — the judge announcing its key, the payout, and the two attempts to peek — as NEAR's <code>tx_status</code> returned them.</li>` +
    `<li><b>Intel's collateral</b> for the quotes, fetched from Intel's PCS. The verifier checks Intel's own signatures on it.</li></ul>` +
    `<h4>Why a saved copy is enough — and how to skip it</h4><p>Every result in it is signed by the judge's key, and every evidence blob ` +
    `hashes to a digest that was signed. What a forged copy could still swap is the key and the root themselves. So read those, and everything else, from NEAR yourself:</p>` +
    `<p><button class=recheck id=vlive>Read everything live from NEAR and check again</button></p><p id=vlive-status class=muted></p>` +
    `<p class=muted>About 30 requests to public NEAR testnet RPC (FastNEAR, then near.org). The same checks also run from a terminal: ` +
    `<code>cargo run --release -p arena-verify -- --arena ${esc(ARENA)}</code>.</p>`).showModal();
  document.getElementById('vlive').addEventListener('click', async (e) => {
    const b = e.target, st = document.getElementById('vlive-status');
    b.disabled = true;
    st.textContent = 'Reading NEAR…';
    try {
      render(await run(true));
      st.innerHTML = document.querySelector('.verify-summary').innerHTML;
    } catch (err) {
      st.textContent = `${err.message}. The check of the saved copy still stands.`;
    }
    b.disabled = false;
  });
}
document.addEventListener('click', (e) => {
  const t = e.target.closest('[data-open], .vhow');
  if (!t) return;
  if (t.classList.contains('vhow')) openHow(); else if (last) openClaim(t.dataset.open);
});
async function start() {
  try {
    render(await run(false));
  } catch (e) {
    document.querySelectorAll('.verify-summary').forEach(el => { el.textContent = 'The check could not run: ' + e.message; });
  }
}
window.arenaVerify = { run: (live = false) => run(live).then(render), claims: CLAIMS };
start();
