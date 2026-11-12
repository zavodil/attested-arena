#!/usr/bin/env python3
"""Build the season report: web/index.html and web/evidence/, from NEAR state.

    scripts/report.py <season> [--verify-output FILE]

Everything on the page is read from the arena contract at build time and
saved under web/evidence/ next to it, so the page is a snapshot anyone can
diff against the chain. Transaction hashes come from scripts/.state/.
"""
import html
import json
import re
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import season as s  # noqa: E402

ROOT = s.ROOT
WEB = os.path.join(ROOT, "web")
EXPLORER = "https://testnet.nearblocks.io"
REPO = "https://github.com/zavodil/attested-arena"


def e(x):
    return html.escape(str(x))


def short(h, n=10):
    h = str(h)
    return h if len(h) <= 2 * n else f"{h[:n]}…{h[-6:]}"


def tx_link(h, label=None):
    return f'<a href="{EXPLORER}/txns/{e(h)}" title="{e(h)}"><code>{e(label or short(h, 8))}</code></a>'


def acct_link(a):
    return f'<a href="{EXPLORER}/address/{e(a)}"><code>{e(a)}</code></a>'


def paged(acct, method, total, page=50):
    out, i = [], 0
    while i < total:
        out += s.view(acct, method, {"from": i, "limit": page})
        i += page
    return out


def txs(season):
    path = os.path.join(ROOT, "scripts", ".state", f"{season}-txs.jsonl")
    if not os.path.exists(path):
        return []
    return [json.loads(l) for l in open(path) if l.strip()]


def first_tx(log, method):
    return next((t["hash"] for t in log if t["method"] == method and t["status"] == "ok"), None)


def model_short(m):
    return m.split("/")[-1]


CSS = """
:root { --paper:#f6f2ea; --card:#fffdf8; --ink:#1d1b16; --muted:#6b6458; --line:#e4dccd; --accent:#c8442b; --accent-soft:#f6e1d9;
        --ok:#2f6f3e; --bad:#a33224; --gold:#b8862b; }
* { box-sizing:border-box } html { scroll-behavior:smooth }
body { margin:0; font:17px/1.6 "IBM Plex Sans",-apple-system,"Segoe UI",sans-serif; color:var(--ink); background:var(--paper) }
main { max-width:1060px; margin:0 auto; padding:0 28px 96px }
h1, h2, h3 { font-family:"Fraunces",Georgia,serif; font-weight:600; letter-spacing:-0.01em }
h2 { font-size:34px; margin:72px 0 10px; } h2 small { display:block; font:600 13px/1 "IBM Plex Mono",monospace; letter-spacing:.12em; text-transform:uppercase; color:var(--accent); margin-bottom:10px }
h3 { font-size:22px; margin:30px 0 8px }
p { max-width:780px } code, .mono { font:14px/1.4 "IBM Plex Mono",ui-monospace,Menlo,monospace } a { color:var(--accent); text-decoration:none } a:hover { text-decoration:underline }
.hero { padding:72px 0 28px; border-bottom:1px solid var(--line) }
.hero .kicker { font:600 13px/1 "IBM Plex Mono",monospace; letter-spacing:.14em; text-transform:uppercase; color:var(--accent) }
.hero h1 { font-size:68px; line-height:1.02; margin:14px 0 18px; max-width:900px }
.hero p.lead { font-size:22px; line-height:1.45; color:#3b372f; max-width:860px; margin:0 0 22px }
.hero .meta { color:var(--muted); font-size:15px }
.tiles { display:grid; grid-template-columns:repeat(auto-fit,minmax(170px,1fr)); gap:0; margin:34px 0 8px; border:1px solid var(--line); background:var(--card) }
.tile { padding:18px 20px; border-right:1px solid var(--line); border-bottom:1px solid var(--line) } .tile b { display:block; font:600 34px/1.1 "Fraunces",Georgia,serif } .tile span { color:var(--muted); font-size:14px }
.flow { margin:30px 0 6px } .flow svg { width:100%; height:auto }
.cards { display:grid; grid-template-columns:repeat(auto-fit,minmax(290px,1fr)); gap:16px; margin:18px 0 }
.card { background:var(--card); border:1px solid var(--line); padding:20px 22px } .card h4 { font:600 19px/1.3 "Fraunces",Georgia,serif; margin:0 0 6px } .card p { margin:0; font-size:15.5px; color:#3b372f }
.pull { border-left:4px solid var(--accent); padding:6px 0 6px 22px; margin:26px 0; font:500 24px/1.4 "Fraunces",Georgia,serif; max-width:860px }
table { border-collapse:collapse; width:100%; background:var(--card); border:1px solid var(--line); margin:12px 0 }
th, td { text-align:left; padding:10px 12px; border-bottom:1px solid var(--line); vertical-align:top; font-size:15px } th { font:600 12px/1.2 "IBM Plex Mono",monospace; letter-spacing:.06em; text-transform:uppercase; color:var(--muted); background:#f1ebdf }
.ok { color:var(--ok); font-weight:600 } .bad { color:var(--bad); font-weight:600 } .muted { color:var(--muted) }
details { background:var(--card); border:1px solid var(--line); margin:10px 0; padding:10px 16px } summary { cursor:pointer; font-weight:600 }
pre { background:#1d1b16; color:#efe9dc; padding:16px 18px; overflow:auto; font:13px/1.5 "IBM Plex Mono",ui-monospace,Menlo,monospace; white-space:pre-wrap }
.callout { background:var(--accent-soft); border:1px solid #ebc4b8; padding:14px 18px; margin:16px 0; max-width:none }
.shot { margin:22px 0 } .shot img { max-width:100%; border:1px solid var(--line); background:#fff } .shot figcaption { color:var(--muted); font-size:14px; margin-top:6px }
.step { display:inline-flex; align-items:center; justify-content:center; background:var(--ink); color:var(--paper); width:30px; height:30px; font:600 14px "IBM Plex Mono",monospace; margin-right:10px }
ol.steps { padding-left:0; list-style:none } ol.steps li { margin:14px 0; display:flex; gap:12px; align-items:flex-start } ol.steps li .step { flex:none; margin:0 }
td code, td .mono { word-break:break-all }
footer { border-top:1px solid var(--line); margin-top:80px; padding-top:22px; color:var(--muted); font-size:14px }
"""


CHOICES = [
    ("The judge runs from its wasm URL, not as a project", "A project's keys survive upgrades, so its owner could ship a version that dumps the pool. Keys derived from the build's sha256 die with the build."),
    ("The arena is locked, and the judge checks it", "The contract decides which tasks a portion may open. With no key on its account nobody can swap that code mid-season, and the judge refuses an arena that still has one."),
    ("Code designs the task; a model only writes words", "Models cannot reliably write conditions consistent with their own answer — in our first test 5 of 6 failed their own check. Code guarantees exactly one answer; a second model checks the wording."),
    ("Admission borrowed from olympiad problem setting", "Codeforces Polygon and the Kattis/ICPC format already solved 'is this task sound': the author's solution passes, known wrong answers fail, damaged solutions fail, a reader understands the same task."),
    ("Each task used once, revealed at once", "A fixed hidden set leaks through its scores. A one-use task leaks nothing about the next one, so it can be published immediately."),
    ("Paired duels, SPRT, confirmation", "Generated pools drift in difficulty; only a paired comparison is immune. SPRT stops as soon as the evidence is enough. Confirmation on fresh tasks defeats picking the luckiest of many tries."),
    ("Gain measured on confirmation tasks only", "The tasks that decided whether a challenger got a confirmation are biased in its favour; the ones after are not."),
    ("The contract trusts only a signature", "No oracle, no relay: the judge's signed result arrives in the arena's own callback through OutLayer's yield/resume, and the contract checks the build's key."),
    ("Reproducible builds", "The judge's sha256 is its identity and the contract's hash is what NEAR runs; both rebuild to the same bytes from the repository."),
]

FLOW_SVG = """<svg viewBox='0 0 1000 150' xmlns='http://www.w3.org/2000/svg' font-family='IBM Plex Sans, sans-serif'>
<defs><marker id='ar' viewBox='0 0 10 10' refX='9' refY='5' markerWidth='7' markerHeight='7' orient='auto'><path d='M0,0 L10,5 L0,10 z' fill='#c8442b'/></marker></defs>
""" + "".join(
    f"<g transform='translate({20 + i * 196},20)'><rect width='170' height='104' fill='{'#1d1b16' if i in (0, 2) else '#fffdf8'}' stroke='#1d1b16'/>"
    f"<text x='14' y='30' font-size='12' font-family='IBM Plex Mono, monospace' fill='#c8442b'>0{i + 1}</text>"
    f"<text x='14' y='58' font-size='17' font-weight='600' font-family='Fraunces, Georgia, serif' fill='{'#f6f2ea' if i in (0, 2) else '#1d1b16'}'>{t1}</text>"
    f"<text x='14' y='82' font-size='13' fill='{'#d9d2c4' if i in (0, 2) else '#6b6458'}'>{t2}</text></g>"
    + (f"<line x1='{190 + i * 196}' y1='72' x2='{214 + i * 196}' y2='72' stroke='#c8442b' stroke-width='2' marker-end='url(#ar)'/>" if i < 4 else "")
    for i, (t1, t2) in enumerate([("Written in a TEE", "code designs it"), ("Sealed, committed", "Merkle root on NEAR"),
                                  ("Dueled in TEEs", "NEAR AI, signed answers"), ("Revealed at once", "re-checkable by anyone"),
                                  ("Paid by contract", "on the judge's signature")])) + "</svg>"


def answer_text(resp_json):
    try:
        c = json.loads(resp_json)["choices"][0]["message"]["content"] or ""
    except Exception:
        return "(no content)"
    if "</think>" in c:
        c = c.rsplit("</think>", 1)[1]
    i = c.rfind("<answer>")
    if i >= 0:
        j = c.find("</answer>", i)
        return c[i + 8:j if j > 0 else None].strip()
    return "(no <answer> block) " + c.strip()[-200:]


def main():
    season = sys.argv[1]
    verify_out = sys.argv[sys.argv.index("--verify-output") + 1] if "--verify-output" in sys.argv else None
    acct = s.arena(season)
    st = s.view(acct, "get_season")
    state = json.load(open(s.state_path(season)))
    log = txs(season)
    batches = paged(acct, "get_batches", st["batches"])
    duels = paged(acct, "get_duels", st["duels"])
    portions = paged(acct, "get_portions", st["portions"], 20)
    entries = paged(acct, "get_entries", st["entries"])
    reveals = paged(acct, "get_reveals", st["reveals"])
    evidence = {}
    for i, p in enumerate(portions):
        if "Done" in p["state"]:
            evidence[f"p{i}"] = s.view(acct, "get_evidence", {"key": f"p{i}"})
    exp_path = os.path.join(ROOT, "scripts", ".state", f"experiments-{season}.json")
    experiments = json.load(open(exp_path)) if os.path.exists(exp_path) else None
    ft_supply = st["closed_supply"] if st["status"] == "Closed" else st["token_supply"]
    revealed_after = sum(len(r["tasks"]) for r in reveals if "Done" in r["state"])

    os.makedirs(os.path.join(WEB, "evidence"), exist_ok=True)
    for name, data in [("season", st), ("batches", batches), ("duels", duels), ("portions", portions), ("entries", entries),
                       ("reveals", reveals), ("portion-evidence", evidence), ("state", state), ("transactions", log),
                       ("experiments", experiments)]:
        json.dump(data, open(os.path.join(WEB, "evidence", f"{name}.json"), "w"), indent=1)

    generated = sum(b["generated"] for b in batches if "Done" in b["state"])
    admitted = sum(b["admitted"] for b in batches if "Done" in b["state"])
    rejected = {}
    for b in batches:
        for k, v in b["rejected"]:
            rejected[k] = rejected.get(k, 0) + v
    used = sum(p["count"] for p in portions if "Done" in p["state"])
    answers = 2 * used
    records = [d for d in duels if isinstance(d["outcome"], dict) and "Record" in d["outcome"]]

    h = []
    h.append(f"<!doctype html><html lang=en><head><meta charset=utf-8><meta name=viewport content='width=device-width,initial-scale=1'>"
             f"<title>Attested Arena — a benchmark nobody has seen</title>"
             "<meta name=description content='A model benchmark whose test set nobody has ever seen — not the developers, not the organiser — and whose every score anyone can re-check. A season on NEAR testnet.'>"
             "<link rel=preconnect href=https://fonts.googleapis.com><link rel=preconnect href=https://fonts.gstatic.com crossorigin>"
             "<link href='https://fonts.googleapis.com/css2?family=Fraunces:opsz,wght@9..144,500;9..144,600&family=IBM+Plex+Mono:wght@400;600&family=IBM+Plex+Sans:wght@400;500;600&display=swap' rel=stylesheet>"
             f"<style>{CSS}</style></head><body><main>")
    h.append("<header class=hero><div class=kicker>NEAR HACKATHON · Privacy &amp; Private AI</div>"
             "<h1>Attested Arena</h1>"
             "<p class=lead>A model benchmark whose test set nobody has ever seen — not the model developers, not the organiser, not us — "
             "and whose every score anyone can re-check. Sponsors fund it; proven improvements get paid.</p>"
             f"<p class=meta>This page is a snapshot of {acct_link(acct)} on NEAR testnet, saved with its evidence under "
             f"<a href='{REPO}/tree/main/web/evidence'>evidence/</a> · source and the checker: <a href='{REPO}'>{REPO.split('//')[1]}</a></p></header>")
    h.append("<div class=tiles>" + "".join(
        f"<div class=tile><b>{v}</b><span>{k}</span></div>" for k, v in [
            ("tasks written in the TEE", generated), ("admitted by code", admitted), ("sealed in the pool", st["pool_tasks"]),
            ("tasks used once, then revealed", used), ("answers proven & re-scored", answers), ("duels", len(duels)),
            ("never-opened tasks revealed after the season", revealed_after), ("confirmed records", len(records)),
            ("progress tokens minted", f"{int(ft_supply) / 100:.2f}")]) + "</div>")

    h.append("<div class=flow>" + FLOW_SVG + "</div>")

    h.append("<h2><small>Why</small>How do you trust a score?</h2>")
    h.append("<p>A community just took a 0.8B model from 17.4% to 53.4% in one week by editing its weights "
             "(<a href='https://deltaleague.ai'>Delta League</a>). Open, cheap model improvement like that only works if anyone can trust "
             "the result — which means a private test set that no one has seen, that is not on the internet, and that runs on a model "
             "without being revealed to its developer. Today every private set has an owner who has seen it.</p>")
    h.append("<div class=cards>"
             "<div class=card><h4>Test sets leak</h4><p>Public benchmarks end up in training data. A private set leaks by design when it is "
             "run on a closed model: evaluating it means sending every question to the model's vendor.</p></div>"
             "<div class=card><h4>The holder is trusted on their word</h4><p>Whoever holds the hidden set saw it, can change it, and cannot "
             "prove otherwise. FrontierMath's problems were funded by a lab it ranks, which had access to most of them.</p></div>"
             "<div class=card><h4>Leaderboards get fitted</h4><p>Each submission against a fixed hidden set returns a score; picking the best "
             "of many tries starts to look like progress. On 500 tasks the standard error is about 2 points.</p></div></div>")
    h.append("<p class=pull>Attested Arena removes the one person who has seen the test.</p>")
    h.append("<p>The tasks are created inside a TEE, sealed to the exact build of the code that created them, and committed on NEAR before "
             "the first duel. Models answer them inside NEAR AI's confidential inference. Every task is used once and revealed with its result. "
             "Sponsors put money behind a season, and a model that proves it is better is paid by a contract that trusts only the judge's "
             "signature. It is the neutral ground for <b>verifiable eval-as-a-service</b>: a company can fund a season on its own kind of "
             "problem and pay whoever improves a model at it — and nobody, the company included, can leak or tune the test.</p>")

    # The idea in steps
    h.append("<h2><small>The season</small>What happened on NEAR testnet</h2>")
    steps = [
        ("The judge was published and pinned.", f"Build <code>{short(state['judge_sha256'])}</code>, key <code>{short(state['judge_key'])}</code>, said by the build itself in {tx_link(state['identity_tx'])}."),
        ("The arena was deployed and locked.", f"{acct_link(acct)} has no full-access key: its code — which decides which tasks a portion may open — cannot change."),
        ("The exam was written inside the TEE.", f"{len(batches)} batches: {generated} tasks written, {admitted} admitted by code, each sealed to the judge's build. Nobody saw them."),
        ("The pool was committed.", f"Merkle root <code>{short(st['root'], 12)}</code> over {st['pool_tasks']} leaves, fixed by {tx_link(first_tx(log, 'seal') or '')} before any duel."),
        ("Models dueled.", f"{len(duels)} duels, {sum(1 for p in portions if 'Done' in p['state'])} judged portions, {used} tasks used once and revealed with their results."),
        ("Records were confirmed and paid.", f"{len(records)} confirmed record(s); {int(ft_supply) / 100:.2f} progress tokens minted for the gain measured on confirmation tasks."),
    ]
    h.append("<ol class=steps>" + "".join(
        f"<li><span class=step>{i + 1}</span><span><b>{t}</b> {d}</span></li>" for i, (t, d) in enumerate(steps)) + "</ol>")

    # Judge
    h.append("<h2><small>Step 1</small>The judge</h2>")
    h.append(f"<p>The judge is one WebAssembly module, run by OutLayer in an Intel TDX enclave directly from its URL — never as an upgradable project. "
             f"OutLayer's keystore derives its keys from the build's sha256 (<code>bind: \"wasm\"</code>): another build, ours included, gets other keys.</p>")
    h.append("<table><tr><th>Build (sha256)</th><th>Judge key</th></tr>" +
             f"<tr><td style='width:50%'><code>{e(state['judge_sha256'])}</code><br><a href='{e(s.judge_source(state['judge_sha256'])['url'])}'>wasm on FastFS</a></td><td><code>{e(state['judge_key'])}</code></td></tr></table>")

    # Pool
    h.append("<h2><small>Step 2</small>The exam nobody saw</h2>")
    h.append("<p>Each batch: code designed the tasks from the enclave's randomness, a model in NEAR AI's confidential inference wrote the wording, "
             "and code decided admission. Only ciphertexts and leaf hashes left the enclave.</p>")
    rows = "".join(
        f"<tr><td>{i}</td><td>{e(b['family'])}</td><td>{b['generated']}</td><td>{b['admitted']}</td><td>{e(dict(b['rejected']) or '—')}</td></tr>"
        for i, b in enumerate(batches) if "Done" in b["state"])
    fam = {}
    for b in batches:
        if "Done" in b["state"]:
            f = fam.setdefault(b["family"], [0, 0, 0])
            f[0] += 1; f[1] += b["generated"]; f[2] += b["admitted"]
    h.append("<table><tr><th>Family</th><th>Batches</th><th>Written</th><th>Admitted</th></tr>" + "".join(
        f"<tr><td>{e(k)}</td><td>{v[0]}</td><td>{v[1]}</td><td>{v[2]}</td></tr>" for k, v in fam.items()) + "</table>")
    h.append(f"<details><summary>Every batch, with its signed admission counts</summary><table><tr><th>Batch</th><th>Family</th><th>Written</th><th>Admitted</th><th>Kept out</th></tr>{rows}</table></details>")
    if rejected:
        h.append(f"<p class=muted>Kept out over the season: {e(rejected)}.</p>")

    # Duels
    h.append("<h2><small>Step 3</small>Duels</h2>")
    h.append("<p>Both models answer the same tasks; a sequential test (SPRT, α 5%, β 10%) decides on the tasks where they disagree; "
             "a passed test earns a confirmation on fresh tasks, and the gain is measured there. "
             "A run that fails — here, runs that outlived OutLayer's 200-block window for returning a result — spends its tasks unopened: "
             "they may have reached a model, so they are never used again.</p>")
    for d_i, d in enumerate(duels):
        oc = d["outcome"]
        verdict = (f"<span class=ok>record, +{oc['Record']['gain_centi'] / 100:.2f} points</span>" if isinstance(oc, dict)
                   else f"<span class=bad>{e(oc)}</span>" if oc else "running")
        entry = entries[d["entry"]]
        h.append(f"<h3>Duel {d_i}: {e(model_short(d['challenger_model']))} (entered by {acct_link(entry['account'])}) "
                 f"vs champion {e(model_short(d['champion_model']))} — {verdict}</h3>")
        if d["challenger_model"] == d["champion_model"]:
            judged = [portions[pid] for pid in d["portions"] if "Done" in portions[pid]["state"]]
            n = sum(p["count"] for p in judged)
            diff = sum(1 for p in judged for a, b in p["results"] if a != b)
            h.append(f"<p class=callout>The champion's own model, entered again. At temperature 0 the same model still answered "
                     f"{diff} of {n} tasks differently from itself (GPU batching is not bit-exact, and long reasoning hits the token "
                     f"budget at different points). The test treats that as noise: no record, nothing minted.</p>")
        if isinstance(oc, dict):
            h.append(f"<p>{acct_link(entry['account'])} received {oc['Record']['gain_centi'] / 100:.2f} progress tokens: the gain on the confirmation tasks only.</p>")
        prow = []
        for pid in d["portions"]:
            p = portions[pid]
            if "Done" in p["state"]:
                c = sum(1 for a, _ in p["results"] if a)
                x = sum(1 for _, b in p["results"] if b)
                prow.append(f"<tr><td>{pid}</td><td>{e(p['phase'])}</td><td>{p['first_task']}–{p['first_task'] + p['count'] - 1}</td>"
                            f"<td>{c}/{p['count']}</td><td>{x}/{p['count']}</td><td class=ok>signed by the judge</td></tr>")
            else:
                why = p["state"].get("Failed", "pending") if isinstance(p["state"], dict) else p["state"]
                prow.append(f"<tr><td>{pid}</td><td>{e(p['phase'])}</td><td>{p['first_task']}–{p['first_task'] + p['count'] - 1}</td>"
                            f"<td colspan=2 class=muted>no result: tasks spent unopened</td><td class=muted>{e(str(why)[:90])}</td></tr>")
        h.append("<table><tr><th>Portion</th><th>Phase</th><th>Tasks</th><th>Champion right</th><th>Challenger right</th><th></th></tr>" + "".join(prow) + "</table>")
        # one portion's tasks, opened
        for pid in d["portions"][:1]:
            ev = evidence.get(f"p{pid}")
            if not ev:
                continue
            ev = json.loads(ev)
            for j, tj in enumerate(ev["tasks"][:2]):
                task = json.loads(tj)
                prompt = task.get("statement", "")
                ans = [a for a in ev["answers"] if a["task"] == j]
                rows = "".join(
                    f"<tr><td>{e(model_short(a['model']))}</td><td><pre style='margin:0;background:#f6f7f9;color:#222'>{e(answer_text(a['response']))[:900]}</pre></td>"
                    f"<td class={'ok' if a['verdict']['ok'] else 'bad'}>{'right' if a['verdict']['ok'] else e(a['verdict'].get('reason', 'wrong'))}</td>"
                    f"<td><code>{e(short(a['signature']['signing_address'], 6))}</code></td></tr>" for a in ans)
                h.append(f"<details{' open' if j == 0 and d_i == 0 else ''}><summary>Task {portions[pid]['first_task'] + j} ({e(task['family'])}) — revealed after portion {pid}</summary>"
                         f"<pre>{e(prompt)}</pre><table><tr><th>Model</th><th>Answer</th><th>Checker</th><th>TEE signer</th></tr>{rows}</table></details>")

    # Experiments
    h.append("<h2><small>Attacks</small>Trying to get around it</h2>")
    if experiments:
        d1 = experiments["direct"]
        d2 = experiments["other_build"]
        h.append("<table><tr><th>Attempt</th><th>What happened</th><th>Transaction</th></tr>"
                 f"<tr><td>The operator calls the judge's own build directly, not through the arena, asking it to open a sealed task that was never opened.</td>"
                 f"<td class=ok>Refused: <code>{e(json.dumps(d1['output'])[:220])}</code></td><td>{tx_link(d1['tx'])}</td></tr>"
                 f"<tr><td>The operator runs another build that declares the judge's exact <code>pool</code> key (<code>bind: wasm</code>, <code>caller: signer</code>) on the same ciphertext with the same <code>aad</code>.</td>"
                 f"<td class=ok>Does not open: <code>{e(json.dumps(d2['output'])[:220])}</code></td><td>{tx_link(d2['tx'])}</td></tr>"
                 "</table>")
    h.append("<p>In the contract's tests (<code>contract/src/tests.rs</code>): a result signed by any other build is refused; a result for another arena or "
             "operator is refused; evidence other than the signed evidence is refused; a portion must score exactly the tasks it was given; entering "
             "the champion's own model earns nothing; a challenger that passes by luck fails the confirmation.</p>")

    # Money
    h.append("<h2><small>Step 4</small>Money</h2>")
    pool = int(st["closed_pool_yocto"] if st["status"] == "Closed" else st["pool_yocto"]) / 1e24
    h.append(f"<p>Sponsors put {pool:.2f} NEAR into the season's pool. Each path out is one rule: <code>redeem</code> pays "
             f"<code>balance × pool / supply</code> (both fixed at close) and burns the tokens; with no record, <code>reclaim</code> "
             f"returns each sponsor's own deposit; <code>refund_deposit</code> returns every entry's deposit, whatever its duel did.</p>")
    rows = []
    for t in log:
        if t["method"] in ("sponsor", "redeem", "reclaim", "refund_deposit", "enter", "close") and t["status"] == "ok":
            what = {"sponsor": "put into the pool", "redeem": "redeemed all progress tokens for its share of the pool",
                    "reclaim": "took back its sponsorship", "refund_deposit": "took back its entry deposit",
                    "enter": f"entered {e(t['args'].get('model', ''))}", "close": "closed the season"}[t["method"]]
            rows.append(f"<tr><td>{acct_link(t['signer'])}</td><td><code>{e(t['method'])}</code> — {what}</td><td>{tx_link(t['hash'])}</td></tr>")
    h.append("<table><tr><th>Account</th><th>Call</th><th>Transaction</th></tr>" + "".join(rows) + "</table>")

    h.append("<h2><small>Design</small>Why each choice</h2><table><tr><th>Choice</th><th>Why</th></tr>" + "".join(
        f"<tr><td><b>{a}</b></td><td>{b}</td></tr>" for a, b in CHOICES) + "</table>")
    h.append("<h2><small>Next</small>Weights, not just models</h2>"
             "<p>This season's duels run between NEAR AI's catalog models, because its confidential inference serves its own catalog only. "
             "The arena is written for Delta League's format (<code>delta.submission.v1</code>, a version named by the hash of its content). "
             "One step is missing: if NEAR AI serves an uploaded weight delta under its content hash, the response signature it already makes — "
             "<code>model_id:sha256(request):sha256(response)</code> — binds every answer to exact weights, and Delta League gets a private, "
             "verifiable, paid eval with no change to the arena or the judge.</p>")
    h.append("<h2><small>Trust</small>What you still trust</h2><ul>"
             "<li><b>The hardware</b>: Intel TDX and NVIDIA confidential GPUs, verified against Intel's and NVIDIA's services. Physical attacks on a host are outside Intel's model.</li>"
             "<li><b>OutLayer's keystore and workers</b> (both in TDX, measurements approved on NEAR) for deriving a build's keys only for that build and reporting the run's caller truthfully.</li>"
             "<li><b>NEAR AI's deployment</b> for which weights stand behind a model name: a quote attests the signing key and the software stack, not a hash of the weights.</li>"
             "</ul><p>Not the operator, not the organiser, not this page: every number here is re-checked by <code>arena-verify</code> from NEAR state.</p>")

    # Verify
    if verify_out and os.path.exists(verify_out):
        h.append("<h2><small>Verify</small>Check it yourself</h2>")
        h.append(f"<pre>$ cargo run --release -p arena-verify -- --arena {e(acct)}\n\n{e(open(verify_out).read())}</pre>")

    # The chain's own answers (01–04); the rest are crops of this page, for the README.
    shots = sorted(f for f in os.listdir(os.path.join(WEB, "shots")) if f[:2] in ("01", "02", "03", "04")) if os.path.isdir(os.path.join(WEB, "shots")) else []
    if shots:
        h.append("<h2><small>On chain</small>What NEAR itself answers</h2>")
        h.append("<p>Raw answers of NEAR RPC for this season, rendered as they came.</p>")
        for f in shots:
            caption = os.path.splitext(f)[0].split("-", 1)[-1].replace("-", " ")
            h.append(f"<figure class=shot><img src='shots/{e(f)}' alt='{e(caption)}'><figcaption>{e(caption)}</figcaption></figure>")
    h.append("<footer>Attested Arena · built at NEAR HACKATHON, November 2026 · "
             f"<a href='{REPO}'>source</a> · our other NEAR HACKATHON project: "
             "<a href='https://github.com/zavodil/near-attested-ai'>Provable AI Compliance</a></footer>")
    h.append("</main></body></html>")
    page = "\n".join(h)
    # Each section gets an id (its heading, slugified), so it can be linked to
    # and captured on its own.
    parts = page.split("<h2>")
    out = parts[0].replace("<div class=tiles>", "<div class=tiles id=glance>", 1)
    for part in parts[1:]:
        title = re.sub(r"<small>.*?</small>", "", part.split("</h2>", 1)[0])
        slug = re.sub(r"[^a-z0-9]+", "-", title.lower()).strip("-")
        out += f"</section><section id={slug}><h2>" + part
    out = out.replace("</section>", "", 1).replace("</main>", "</section></main>")
    open(os.path.join(WEB, "index.html"), "w").write(out)
    print("wrote web/index.html and web/evidence/")


if __name__ == "__main__":
    main()
