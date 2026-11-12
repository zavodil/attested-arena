#!/usr/bin/env python3
"""Build the season report: web/index.html and web/evidence/, from NEAR state.

    scripts/report.py <season> [--verify-output FILE]

The page states what the season proves and lets the reader re-check every
claim in the browser (web/verify/verify.js reads NEAR testnet live and runs
the judge's own code compiled to WebAssembly). Everything the page shows is
read from the arena contract at build time and saved under web/evidence/.
"""
import html
import json
import os
import re
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
    return f'<a href="{EXPLORER}/txns/{e(h)}" title="{e(h)}">{e(label or short(h, 8))}</a>'


def acct_link(a):
    return f'<a href="{EXPLORER}/address/{e(a)}">{e(a)}</a>'


def paged(acct, method, total, page=50):
    out, i = [], 0
    while i < total:
        out += s.view(acct, method, {"from": i, "limit": page})
        i += page
    return out


def txs(season):
    path = os.path.join(ROOT, "scripts", ".state", f"{season}-txs.jsonl")
    return [json.loads(l) for l in open(path) if l.strip()] if os.path.exists(path) else []


def first_tx(log, method):
    return next((t["hash"] for t in log if t["method"] == method and t["status"] == "ok"), None)


def model_short(m):
    return m.split("/")[-1]


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
    return "(no <answer> block) …" + c.strip()[-200:]


CSS = """
:root { --paper:#f6f2ea; --card:#fffdf8; --ink:#1d1b16; --muted:#6b6458; --line:#e4dccd; --accent:#c8442b; --accent-soft:#f6e1d9;
        --ok:#2f6f3e; --ok-soft:#e3efe2; --bad:#a33224; }
* { box-sizing:border-box } html { scroll-behavior:smooth }
body { margin:0; font:17px/1.6 "IBM Plex Sans",-apple-system,"Segoe UI",sans-serif; color:var(--ink); background:var(--paper) }
main { max-width:1080px; margin:0 auto; padding:0 28px 96px }
h1, h2, h3, h4 { font-family:"Fraunces",Georgia,serif; font-weight:600; letter-spacing:-0.01em }
h2 { font-size:34px; margin:76px 0 10px } h2 small { display:block; font:600 13px/1 "IBM Plex Mono",monospace; letter-spacing:.12em; text-transform:uppercase; color:var(--accent); margin-bottom:10px }
h3 { font-size:22px; margin:30px 0 8px }
p { max-width:800px } a { color:var(--accent); text-decoration:none; font:inherit } a:hover { text-decoration:underline }
code, .mono { font:14px/1.4 "IBM Plex Mono",ui-monospace,Menlo,monospace }
.hero { padding:72px 0 30px }
.kicker { font:600 13px/1 "IBM Plex Mono",monospace; letter-spacing:.14em; text-transform:uppercase; color:var(--accent) }
.hero h1 { font-size:72px; line-height:1; margin:14px 0 10px }
.hero .tag { font:500 30px/1.25 "Fraunces",Georgia,serif; margin:0 0 26px; max-width:900px }
.hero p.lead { font-size:19px; line-height:1.6; color:#2f2b24; max-width:860px }
.values { display:grid; grid-template-columns:repeat(3,1fr); gap:0; margin:30px 0 10px; border:1px solid var(--ink) }
.values div { padding:20px 22px; border-right:1px solid var(--ink); background:var(--card) } .values div:last-child { border-right:0 }
.values h4 { margin:0 0 6px; font-size:20px } .values p { margin:0; font-size:15.5px; color:#3b372f }
.meta { color:var(--muted); font-size:15px; margin-top:18px }
.proofs { display:grid; grid-template-columns:repeat(2,1fr); gap:16px; margin:22px 0 }
.claim { background:var(--card); border:1px solid var(--line); padding:20px 22px; display:flex; flex-direction:column }
.claim h4 { margin:0 0 6px; font-size:21px; line-height:1.25 } .claim .how { font-size:14.5px; color:#3b372f; margin:6px 0 12px; padding-left:18px }
.claim .how li { margin:3px 0 } .claim .result { font-size:14.5px; margin-top:auto; padding-top:10px; border-top:1px dashed var(--line) }
.claim .result ul { margin:6px 0 0; padding-left:18px; color:var(--bad) } .claim.passed { border-color:#9cc2a3; background:var(--ok-soft) } .claim.passed .result b { color:var(--ok) }
.claim.failed { border-color:#e5a99e } .claim details.items { margin:8px 0 0; padding:6px 10px; font-size:13px; background:#fff } .claim details.items ol { margin:6px 0 0; padding-left:20px; max-height:260px; overflow:auto; color:#3b372f } .claim details.items summary { font-weight:500 } .claim.failed .result b { color:var(--bad) }
.claim .tag { font:600 12px/1 "IBM Plex Mono",monospace; letter-spacing:.1em; text-transform:uppercase; color:var(--accent); margin-bottom:8px }
button.recheck { font:600 15px "IBM Plex Sans",sans-serif; background:var(--ink); color:var(--paper); border:0; padding:12px 20px; cursor:pointer }
button.recheck.small { font-size:13px; padding:7px 12px; margin-top:10px; align-self:flex-start; background:transparent; color:var(--ink); border:1px solid var(--ink) }
button.recheck:disabled { opacity:.6; cursor:default } .verify-bar { display:flex; gap:16px; align-items:center; flex-wrap:wrap; margin:18px 0 6px }
.verify-summary { font-size:15px; color:var(--muted) }
.proof { background:var(--card); border-left:4px solid var(--accent); padding:14px 20px; margin:18px 0; font-size:15.5px }
.proof b.l { font:600 12px/1 "IBM Plex Mono",monospace; letter-spacing:.1em; text-transform:uppercase; color:var(--accent); display:block; margin-bottom:6px }
.arch { margin:24px 0 8px; background:var(--card); border:1px solid var(--line); padding:10px } .arch svg { width:100%; height:auto; display:block }
.legend { display:grid; grid-template-columns:repeat(2,1fr); gap:6px 26px; margin:14px 0; font-size:15px; padding:0; list-style:none }
.legend li { display:flex; gap:10px } .legend span { flex:none; width:24px; height:24px; border-radius:50%; background:var(--accent); color:#fff; font:600 13px/24px "IBM Plex Mono",monospace; text-align:center }
.legend li:last-child span { background:var(--ink) }
table { border-collapse:collapse; width:100%; background:var(--card); border:1px solid var(--line); margin:12px 0 }
th, td { text-align:left; padding:10px 12px; border-bottom:1px solid var(--line); vertical-align:top; font-size:15px }
th { font:600 12px/1.2 "IBM Plex Mono",monospace; letter-spacing:.06em; text-transform:uppercase; color:var(--muted); background:#f1ebdf }
td code { word-break:break-all }
.ok { color:var(--ok); font-weight:600 } .bad { color:var(--bad); font-weight:600 } .muted { color:var(--muted) }
details { background:var(--card); border:1px solid var(--line); margin:10px 0; padding:10px 16px } summary { cursor:pointer; font-weight:600 }
pre { background:#1d1b16; color:#efe9dc; padding:16px 18px; overflow:auto; font:13px/1.5 "IBM Plex Mono",ui-monospace,Menlo,monospace; white-space:pre-wrap }
td pre { background:#f3eee4; color:#222; margin:0; padding:8px 10px }
.callout { background:var(--accent-soft); border:1px solid #ebc4b8; padding:14px 18px; margin:16px 0 }
ol.steps { padding-left:0; list-style:none } ol.steps li { margin:14px 0; display:flex; gap:12px } .step { flex:none; display:inline-flex; align-items:center; justify-content:center; background:var(--ink); color:var(--paper); width:30px; height:30px; font:600 14px "IBM Plex Mono",monospace }
.shot { margin:22px 0 } .shot img { max-width:100%; border:1px solid var(--line); background:#fff } .shot figcaption { color:var(--muted); font-size:14px; margin-top:6px }
footer { border-top:1px solid var(--line); margin-top:80px; padding-top:22px; color:var(--muted); font-size:14px }
@media (max-width:800px) { .values, .proofs, .legend { grid-template-columns:1fr } .hero h1 { font-size:52px } }
"""

# The architecture: who talks to whom, and where the trust boundaries are.
ARCH_SVG = """<svg viewBox="0 0 1100 640" xmlns="http://www.w3.org/2000/svg" font-family="'IBM Plex Sans', 'Helvetica Neue', Arial, sans-serif" font-size="13" fill="#1d1b16">
<defs>
 <marker id="ah" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto-start-reverse"><path d="M0,0 L10,5 L0,10 z" fill="#1d1b16"/></marker>
</defs>
<style>
 .zone{font-family:'IBM Plex Mono',Menlo,monospace;font-size:12px;font-weight:600;letter-spacing:.12em;fill:#c8442b}
 .zsub{font-size:12.5px;fill:#6b6458}
 .h{font-family:Fraunces,Georgia,serif;font-weight:600;font-size:20px}
 .hp{font-family:Fraunces,Georgia,serif;font-weight:600;font-size:18px}
 .mu{font-size:12.5px;fill:#6b6458}
 .b{font-weight:600;font-size:13px}
 .sm{font-size:12px;fill:#6b6458}
 .lab{font-size:12px;fill:#1d1b16}
 .labm{font-size:12px;fill:#6b6458}
 .dk{fill:#f6f2ea}
 .dkm{fill:#cfc6b6;font-size:12.5px}
 .acc{font-family:'IBM Plex Mono',Menlo,monospace;font-size:12px;fill:#eea795}
 .num{font-family:'IBM Plex Mono',Menlo,monospace;font-size:13px;font-weight:600;fill:#fff}
 .arr{stroke:#1d1b16;stroke-width:1.6;fill:none}
</style>

<!-- ============ zone headers ============ -->
<text x="20" y="26" class="zone">PEOPLE</text>
<text x="20" y="44" class="zsub">each one a NEAR account</text>
<text x="330" y="26" class="zone">NEAR BLOCKCHAIN</text>
<text x="330" y="44" class="zsub">the public record and the money</text>
<text x="750" y="26" class="zone">HARDWARE ENCLAVES · TEE</text>
<text x="750" y="44" class="zsub">the neutral ground: nobody can look inside</text>

<!-- ============ people ============ -->
<rect x="20" y="60" width="200" height="72" rx="10" fill="#fffdf8" stroke="#1d1b16"/>
<text x="36" y="88" class="hp">Sponsors</text>
<text x="36" y="110" class="mu">want a better model</text>

<rect x="20" y="160" width="200" height="72" rx="10" fill="#fffdf8" stroke="#1d1b16"/>
<text x="36" y="188" class="hp">Entrants</text>
<text x="36" y="210" class="mu">challenge the champion</text>

<rect x="20" y="290" width="200" height="104" rx="10" fill="#fffdf8" stroke="#1d1b16"/>
<text x="36" y="318" class="hp">Anyone</text>
<text x="36" y="340" class="mu">re-checks every claim</text>
<text x="36" y="357" class="mu">from the chain, with the</text>
<text x="36" y="374" class="mu">judge's own code</text>
<circle cx="220" cy="290" r="12" fill="#c8442b"/><text x="220" y="294.5" text-anchor="middle" class="num">2</text>

<!-- ============ blockchain ============ -->
<rect x="330" y="60" width="300" height="272" fill="#fffdf8" stroke="#1d1b16" stroke-width="2"/>
<text x="348" y="90" class="h">Arena</text>
<rect x="544" y="74" width="70" height="22" rx="11" fill="#c8442b"/>
<text x="579" y="89.5" text-anchor="middle" class="num" style="font-size:12px;letter-spacing:.08em">LOCKED</text>
<text x="348" y="112" class="mu">one smart contract per season;</text>
<text x="348" y="129" class="mu">no access keys, so no rule can change</text>
<line x1="348" y1="142" x2="612" y2="142" stroke="#e4dccd"/>
<text x="348" y="164" class="b">Exam commitment</text>
<text x="348" y="181" class="sm">one Merkle root, fixed before any duel</text>
<text x="348" y="208" class="b">Judge's code hash and key</text>
<text x="348" y="225" class="sm">the only signature it accepts</text>
<text x="348" y="252" class="b">Every result, signed</text>
<text x="348" y="269" class="sm">lost duels too, then each task revealed</text>
<text x="348" y="296" class="b">Prize pool and progress tokens</text>
<text x="348" y="313" class="sm">confirmed gains mint, tokens redeem</text>
<circle cx="630" cy="300" r="12" fill="#c8442b"/><text x="630" y="304.5" text-anchor="middle" class="num">3</text>

<rect x="330" y="350" width="300" height="94" fill="#f6f2ea" stroke="#1d1b16" stroke-dasharray="3 3"/>
<text x="348" y="372" class="zone" style="font-size:12px">WHY A CHAIN</text>
<text x="348" y="392" class="lab">· nobody holds the money</text>
<text x="348" y="410" class="lab">· results public and timestamped</text>
<text x="348" y="428" class="lab">· the judge checks who called it</text>

<!-- ============ enclaves ============ -->
<rect x="738" y="54" width="354" height="502" rx="6" fill="none" stroke="#1d1b16" stroke-dasharray="6 4"/>

<rect x="750" y="66" width="330" height="268" rx="4" fill="#1d1b16"/>
<text x="768" y="96" class="h dk">Judge</text>
<text x="1062" y="94" text-anchor="end" class="acc">on OutLayer · Intel TDX</text>
<text x="768" y="118" class="dkm">a WebAssembly program, run from its</text>
<text x="768" y="135" class="dkm">published file by hash, never upgraded</text>
<line x1="768" y1="147" x2="1062" y2="147" stroke="#4a463d"/>
<g class="dk" font-size="13">
 <text x="768" y="168">· designs tasks from enclave randomness</text>
 <text x="768" y="187">· seals each one, publishes its hash</text>
 <text x="768" y="206">· works only for a keyless arena</text>
 <text x="768" y="225">· checks each model's attestation</text>
 <text x="768" y="244">· scores with open checker code, signs</text>
</g>
<rect x="764" y="258" width="302" height="64" rx="4" fill="none" stroke="#eea795" stroke-width="1.2"/>
<text x="778" y="278" class="b dk">Keys from NEAR MPC network (CKD)</text>
<text x="778" y="296" class="dkm" style="font-size:12px">bound to this code's hash: any other</text>
<text x="778" y="312" class="dkm" style="font-size:12px">version gets other keys, opens nothing</text>
<circle cx="750" cy="66" r="12" fill="#c8442b"/><text x="750" y="70.5" text-anchor="middle" class="num">1</text>

<rect x="750" y="376" width="330" height="168" rx="4" fill="#1d1b16"/>
<text x="768" y="406" class="h dk">Models</text>
<text x="1062" y="404" text-anchor="end" class="acc">NEAR AI</text>
<text x="768" y="428" class="dkm">confidential inference: Intel TDX</text>
<text x="768" y="445" class="dkm">and NVIDIA confidential GPUs</text>
<line x1="768" y1="457" x2="1062" y2="457" stroke="#4a463d"/>
<g class="dk" font-size="13">
 <text x="768" y="478">· one words each task, a second reads it back</text>
 <text x="768" y="497">· champion vs challenger, same tasks</text>
 <text x="768" y="516">· every answer signed inside its TEE</text>
</g>
<text x="768" y="536" class="acc" style="font-size:12px">next: weight deltas, served by hash</text>

<!-- ============ arrows ============ -->
<!-- sponsor -> arena -->
<line x1="222" y1="100" x2="326" y2="100" class="arr" marker-end="url(#ah)"/>
<text x="275" y="92" text-anchor="middle" class="lab">funds the pool</text>
<!-- entrant <-> arena -->
<line x1="222" y1="186" x2="326" y2="186" class="arr" marker-end="url(#ah)"/>
<text x="275" y="178" text-anchor="middle" class="lab">enters a model</text>
<line x1="326" y1="212" x2="222" y2="212" class="arr" marker-end="url(#ah)"/>
<text x="275" y="228" text-anchor="middle" class="labm">paid by rule</text>
<!-- arena -> anyone -->
<line x1="326" y1="320" x2="222" y2="320" class="arr" marker-end="url(#ah)"/>
<text x="275" y="312" text-anchor="middle" class="lab">public record</text>
<!-- arena <-> judge -->
<line x1="632" y1="110" x2="746" y2="110" class="arr" marker-end="url(#ah)"/>
<text x="689" y="102" text-anchor="middle" class="lab">calls the judge</text>
<line x1="746" y1="190" x2="632" y2="190" class="arr" marker-end="url(#ah)"/>
<text x="689" y="182" text-anchor="middle" class="lab">signed result</text>
<text x="689" y="206" text-anchor="middle" class="labm">into its callback</text>
<!-- judge <-> models -->
<line x1="860" y1="336" x2="860" y2="372" class="arr" marker-end="url(#ah)"/>
<text x="850" y="359" text-anchor="end" class="lab">tasks</text>
<line x1="970" y1="372" x2="970" y2="336" class="arr" marker-end="url(#ah)"/>
<text x="980" y="359" class="lab">signed answers</text>

<!-- ============ promises ============ -->
<g transform="translate(20,580)">
 <circle cx="12" cy="18" r="12" fill="#c8442b"/><text x="12" y="22.5" text-anchor="middle" class="num">1</text>
 <text x="32" y="16" class="b">An exam with no keeper</text>
 <text x="32" y="34" class="sm">a test set no one has seen, not even us</text>
</g>
<g transform="translate(385,580)">
 <circle cx="12" cy="18" r="12" fill="#c8442b"/><text x="12" y="22.5" text-anchor="middle" class="num">2</text>
 <text x="32" y="16" class="b">Scores anyone can re-check</text>
 <text x="32" y="34" class="sm">signed in TEEs, re-run from the chain</text>
</g>
<g transform="translate(750,580)">
 <circle cx="12" cy="18" r="12" fill="#c8442b"/><text x="12" y="22.5" text-anchor="middle" class="num">3</text>
 <text x="32" y="16" class="b">Prizes paid by rule</text>
 <text x="32" y="34" class="sm">a bounty paid by a contract with no keys</text>
</g>
<line x1="20" y1="568" x2="1080" y2="568" stroke="#e4dccd"/>
</svg>
"""

CLAIMS = [
    ("exam", "Commitment", "The exam was fixed before the first duel, and every task used came from it",
     ["the Merkle root over the leaves on chain equals the root fixed at <code>seal</code>",
      "every batch's leaves are the ones the judge signed",
      "every revealed task is canonical JSON that hashes to its committed leaf"]),
    ("judge", "Authorship", "Only this build of the judge wrote, sealed and scored it",
     ["the build announced its own key on chain, and the arena pinned that key",
      "every batch, portion and reveal is signed by that key and names this arena and operator",
      "every evidence blob hashes to the digest its result signed"]),
    ("models", "Provenance", "Every answer came from the named model, inside its TEE",
     ["each request is rebuilt byte for byte from the task with the judge's code",
      "each answer's EIP-191 signature covers that request and response and recovers to the model's key",
      "that key's Intel TDX quote verifies against Intel's collateral, is bound to the hash of exactly these requests, and NVIDIA passed its GPUs"]),
    ("scores", "Scores", "Every score is the checker's verdict, not the judge's word",
     ["every answer is scored again in this browser with the judge's own checker code, compiled to WebAssembly",
      "every result must equal what the judge signed"]),
    ("money", "Rules", "Duels, tokens and payouts followed the rules",
     ["each duel's sequential test is replayed from the signed results and must end as on chain",
      "the progress tokens minted equal the confirmed gains",
      "each payout is exactly <code>balance × pool / supply</code>"]),
    ("peek", "Sealing", "Nobody could peek at a task",
     ["the operator calling the judge directly for a sealed task: refused",
      "another build declaring the judge's exact key: the task does not open",
      "the arena account has no key at all"]),
]

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


def proof(claim_ids, text):
    titles = {cid: title for cid, _, title, _ in CLAIMS}
    links = " · ".join(f"<a href='#claim-{c}'>re-check: {titles[c][0].lower() + titles[c][1:]}</a>" for c in claim_ids)
    return f"<div class=proof><b class=l>What is proven here</b>{text}<div style='margin-top:6px'>{links}</div></div>"


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
    evidence = {f"p{i}": s.view(acct, "get_evidence", {"key": f"p{i}"}) for i, p in enumerate(portions) if "Done" in p["state"]}
    exp_path = os.path.join(ROOT, "scripts", ".state", f"experiments-{season}.json")
    experiments = json.load(open(exp_path)) if os.path.exists(exp_path) else None
    minted = int(st["closed_supply"] if st["status"] == "Closed" else st["token_supply"])

    os.makedirs(os.path.join(WEB, "evidence"), exist_ok=True)
    for name, data in [("season", st), ("batches", batches), ("duels", duels), ("portions", portions), ("entries", entries),
                       ("reveals", reveals), ("portion-evidence", evidence), ("state", state), ("transactions", log),
                       ("experiments", experiments)]:
        json.dump(data, open(os.path.join(WEB, "evidence", f"{name}.json"), "w"), indent=1)

    generated = sum(b["generated"] for b in batches if "Done" in b["state"])
    admitted = sum(b["admitted"] for b in batches if "Done" in b["state"])
    used = sum(p["count"] for p in portions if "Done" in p["state"])
    records = [d for d in duels if isinstance(d["outcome"], dict) and "Record" in d["outcome"]]
    pool = int(st["closed_pool_yocto"] if st["status"] == "Closed" else st["pool_yocto"]) / 1e24

    h = []
    h.append(f"<!doctype html><html lang=en data-arena='{e(acct)}'><head><meta charset=utf-8><meta name=viewport content='width=device-width,initial-scale=1'>"
             "<title>Attested Arena — model competitions anyone can fund and everyone can trust</title>"
             "<meta name=description content='A model benchmark whose test set nobody has seen, scored in TEEs, re-checkable in your browser, with prizes paid by contract. A full season on NEAR testnet.'>"
             "<link rel=preconnect href=https://fonts.googleapis.com><link rel=preconnect href=https://fonts.gstatic.com crossorigin>"
             "<link href='https://fonts.googleapis.com/css2?family=Fraunces:opsz,wght@9..144,500;9..144,600&family=IBM+Plex+Mono:wght@400;600&family=IBM+Plex+Sans:wght@400;500;600&display=swap' rel=stylesheet>"
             f"<style>{CSS}</style></head><body><main>")

    h.append("<header class=hero><div class=kicker>NEAR HACKATHON · Privacy &amp; Private AI · AI Security</div>"
             "<h1>Attested Arena</h1>"
             "<p class=tag>Model competitions anyone can fund and everyone can trust.</p>"
             "<p class=lead><a href='https://deltaleague.ai'>Delta League</a> just showed what an open community can do: in one week, people "
             "editing weights took a 0.8B model from 17.4% to 53.4%. The natural next step is to let anyone run competitions like that — on "
             "their own problems, with their own prize money — and still have every result trusted by people who were not in the room. "
             "That takes three things no leaderboard has had together: a test set nobody can leak or tune, scores nobody has to take on "
             "faith, and prizes that pay out by rule.</p>"
             "<div class=values>"
             "<div><h4>An exam with no keeper</h4><p>Tasks are designed and sealed inside a TEE, bound to the exact build of the code that "
             "wrote them. The sponsor, the operator and we cannot read one before it is used — and each is used once.</p></div>"
             "<div><h4>Scores you can re-check</h4><p>Models answer inside NEAR AI's confidential inference and sign every answer there. "
             "Scores come from open checker code. This page re-checks all of it in your browser, from NEAR state.</p></div>"
             "<div><h4>Prizes that pay by rule</h4><p>A sponsor funds a season; a model that proves it is better earns progress tokens "
             "that redeem for the pool — paid by a contract that trusts nothing but the judge's signature.</p></div></div>"
             f"<p class=meta>A full season ran on the NEAR blockchain (testnet): {acct_link(acct)}. Source, tests and the command-line checker: "
             f"<a href='{REPO}'>{REPO.split('//')[1]}</a>. Evidence saved with this page: <a href='{REPO}/tree/main/web/evidence'>web/evidence</a>.</p></header>")

    h.append("<h2><small>Proof</small>What this season proves</h2>")
    h.append("<p>Six claims about the season below. None of them asks you to trust this page: the button reads the arena live from NEAR "
             "testnet and checks every claim in your browser with the judge's own code, compiled to WebAssembly, and Intel's quote "
             "verifier. The same checks run from a terminal with <code>arena-verify</code>.</p>")
    h.append("<div class=verify-bar><button class=recheck>Re-check everything in this browser</button><span class=verify-summary>"
             "Not checked yet. It reads a few megabytes from NEAR and takes about half a minute.</span></div>")
    h.append("<div class=proofs>" + "".join(
        f"<div class=claim id='claim-{cid}' data-claim='{cid}'><div class=tag>{tag}</div><h4>{title}</h4>"
        f"<ul class=how>{''.join(f'<li>{x}</li>' for x in how)}</ul><div class=result><span class=muted>Not checked yet.</span></div>"
        "<button class='recheck small'>Re-check</button></div>" for cid, tag, title, how in CLAIMS) + "</div>")

    h.append("<h2><small>Architecture</small>The exam lives where nobody can read it; the record and the money, where everybody can</h2>")
    h.append("<p>Read it left to right. People are NEAR accounts. They only talk to the arena — one smart contract per season on the NEAR "
             "blockchain, with no access keys, so its rules cannot change. Only the arena calls the judge, and only the judge talks to the "
             "models; both run in hardware enclaves nobody can look inside. The numbers mark where each promise is enforced.</p>")
    h.append("<div class=arch>" + ARCH_SVG + "</div>")
    h.append("<p class=muted>In this build the judge writes the tasks itself. A company's own private test set, sealed to the judge's "
             "code the same way, is the natural next step — the arena does not need to change for it.</p>")

    h.append("<h2><small>The season</small>What happened on NEAR testnet</h2>")
    steps = [
        ("The judge was published and pinned.", f"Build <code>{short(state['judge_sha256'])}</code>, key <code>{short(state['judge_key'])}</code>, said by the build itself in {tx_link(state['identity_tx'])}."),
        ("The arena was deployed and locked.", f"{acct_link(acct)} has no access key: its code — which decides which tasks a portion may open — cannot change."),
        ("The exam was written inside the TEE.", f"{len(batches)} batches: {generated} tasks written, {admitted} admitted by code, each sealed to the judge's build. Nobody saw them."),
        ("The pool was committed.", f"Merkle root <code>{short(st['root'], 12)}</code> over {st['pool_tasks']} leaves, fixed by {tx_link(first_tx(log, 'seal') or '')} before any duel."),
        ("Models dueled.", f"{len(duels)} duels, {sum(1 for p in portions if 'Done' in p['state'])} judged portions, {used} tasks used once and revealed with their results, {2 * used} answers."),
        ("A record was confirmed and paid.", f"{len(records)} confirmed record; {minted / 100:.2f} progress tokens minted for the gain on the confirmation tasks; the sponsors' {pool:.2f} NEAR redeemed by their holder."),
    ]
    h.append("<ol class=steps>" + "".join(f"<li><span class=step>{i + 1}</span><span><b>{t}</b> {d}</span></li>" for i, (t, d) in enumerate(steps)) + "</ol>")

    h.append("<h2><small>Step 1</small>The judge</h2>")
    h.append("<p>The judge is an OutLayer app: a WebAssembly program that runs inside an Intel TDX enclave. It is run straight from its "
             "published file, named by the file's sha256 — never as an upgradable project. Its keys come from the NEAR MPC network's "
             "confidential key derivation (CKD), bound to that hash (<code>bind: \"wasm\"</code>): any other build, ours included, gets "
             "other keys. The build is reproducible: <code>./judge/build.sh</code> gives the same sha256.</p>")
    h.append(f"<table><tr><th>Build (sha256)</th><th>Judge key</th></tr><tr><td style='width:50%'><code>{e(state['judge_sha256'])}</code><br>"
             f"<a href='{e(s.judge_source(state['judge_sha256'])['url'])}'>the wasm on FastFS</a></td><td><code>{e(state['judge_key'])}</code></td></tr></table>")
    h.append(proof(["judge"], "The key the arena pinned is the key this exact build announced in its own on-chain run, and every result "
                   "the arena accepted is signed by it. A later build — even one we publish — signs with a different key and is refused."))

    h.append("<h2><small>Step 2</small>The exam nobody saw</h2>")
    h.append("<p>For each batch, code in the enclave designed the tasks from the enclave's randomness, a model in NEAR AI's confidential "
             "inference wrote the wording, a second model read it back, and code decided admission. Only ciphertexts and leaf hashes left "
             "the enclave; the arena fixed the Merkle root over them before the first duel.</p>")
    fam, rows = {}, []
    for i, b in enumerate(batches):
        if "Done" not in b["state"]:
            continue
        f = fam.setdefault(b["family"], [0, 0, 0])
        f[0] += 1
        f[1] += b["generated"]
        f[2] += b["admitted"]
        rows.append(f"<tr><td>{i}</td><td>{e(b['family'])}</td><td>{b['generated']}</td><td>{b['admitted']}</td><td>{e(dict(b['rejected']) or '—')}</td></tr>")
    h.append("<table><tr><th>Family</th><th>Batches</th><th>Written</th><th>Admitted</th></tr>" + "".join(
        f"<tr><td>{e(k)}</td><td>{v[0]}</td><td>{v[1]}</td><td>{v[2]}</td></tr>" for k, v in fam.items()) + "</table>")
    h.append(f"<details><summary>Every batch, with its signed admission counts</summary><table><tr><th>Batch</th><th>Family</th><th>Written</th><th>Admitted</th><th>Kept out</th></tr>{''.join(rows)}</table></details>")
    h.append(proof(["exam", "peek"], "The pool was committed before any task was used, no task outside it was ever scored, and every task "
                   "is public now and provably one of the committed ones. Before its use, no one could open one — the attempts below failed."))

    h.append("<h2><small>Step 3</small>Duels</h2>")
    h.append("<p>Both models answer the same tasks. A sequential test (SPRT: H1 “the challenger wins 70% of the tasks they disagree on”, "
             "α 5%, β 10%) decides; a passed test earns a confirmation on fresh tasks, and the gain is measured there. A run that fails "
             "— here, one that outlived OutLayer's 200-block window for returning a result — spends its tasks unopened.</p>")
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
                     f"{diff} of {n} tasks differently from itself (GPU batching is not bit-exact). The test treats that as noise: "
                     "no record, nothing minted.</p>")
        if isinstance(oc, dict):
            h.append(f"<p>{acct_link(entry['account'])} received {oc['Record']['gain_centi'] / 100:.2f} progress tokens: the gain on the confirmation tasks only.</p>")
        prow = []
        for pid in d["portions"]:
            p = portions[pid]
            span = f"{p['first_task']}–{p['first_task'] + p['count'] - 1}"
            if "Done" in p["state"]:
                c = sum(1 for a, _ in p["results"] if a)
                x = sum(1 for _, b in p["results"] if b)
                prow.append(f"<tr><td>{pid}</td><td>{e(p['phase'])}</td><td>{span}</td><td>{c}/{p['count']}</td><td>{x}/{p['count']}</td><td class=ok>signed by the judge</td></tr>")
            else:
                why = p["state"].get("Failed", "pending") if isinstance(p["state"], dict) else p["state"]
                prow.append(f"<tr><td>{pid}</td><td>{e(p['phase'])}</td><td>{span}</td><td colspan=2 class=muted>no result: tasks spent unopened</td><td class=muted>{e(str(why)[:90])}</td></tr>")
        h.append("<table><tr><th>Portion</th><th>Phase</th><th>Tasks</th><th>Champion right</th><th>Challenger right</th><th></th></tr>" + "".join(prow) + "</table>")
        for pid in d["portions"][:1]:
            ev = evidence.get(f"p{pid}")
            if not ev:
                continue
            ev = json.loads(ev)
            for j, tj in enumerate(ev["tasks"][:2]):
                task = json.loads(tj)
                ans = [a for a in ev["answers"] if a["task"] == j]
                arows = "".join(
                    f"<tr><td>{e(model_short(a['model']))}</td><td><pre>{e(answer_text(a['response']))[:900]}</pre></td>"
                    f"<td class={'ok' if a['verdict']['ok'] else 'bad'}>{'right' if a['verdict']['ok'] else e(a['verdict'].get('reason', 'wrong'))}</td>"
                    f"<td><code>{e(short(a['signature']['signing_address'], 6))}</code></td></tr>" for a in ans)
                h.append(f"<details{' open' if j == 0 and d_i == 0 else ''}><summary>Task {portions[pid]['first_task'] + j} ({e(task['family'])}) — revealed after portion {pid}</summary>"
                         f"<pre>{e(task.get('statement', ''))}</pre><table><tr><th>Model</th><th>Answer</th><th>Checker</th><th>TEE signer</th></tr>{arows}</table></details>")
    h.append(proof(["models", "scores", "money"], "Every answer in every duel was produced by the named model inside its TEE (Intel TDX "
                   "quote and NVIDIA verdict, bound to the hash of exactly these requests), every score is what the open checker says, "
                   "and every duel ended where the test, replayed from the signed results, ends."))

    h.append("<h2><small>Attacks</small>Trying to get around it</h2>")
    if experiments:
        d1, d2 = experiments["direct"], experiments["other_build"]
        h.append("<table><tr><th>Attempt</th><th>What happened</th><th>Transaction</th></tr>"
                 "<tr><td>The operator calls the judge's own build directly, not through the arena, asking it to open a sealed task that was never opened.</td>"
                 f"<td class=ok>Refused: <code>{e(json.dumps(d1['output'])[:220])}</code></td><td>{tx_link(d1['tx'])}</td></tr>"
                 "<tr><td>The operator runs another build that declares the judge's exact <code>pool</code> key (<code>bind: wasm</code>, <code>caller: signer</code>) on the same ciphertext with the same <code>aad</code>.</td>"
                 f"<td class=ok>Does not open: <code>{e(json.dumps(d2['output'])[:220])}</code></td><td>{tx_link(d2['tx'])}</td></tr></table>")
    h.append("<p>And in the contract's tests (<code>contract/src/tests.rs</code>): a result signed by any other build is refused; a result for "
             "another arena or operator is refused; evidence other than the signed evidence is refused; a portion must score exactly the "
             "tasks it was given; entering the champion's own model earns nothing; a challenger that passes by luck fails the confirmation.</p>")
    h.append(proof(["peek"], "The operator — the one party that runs the season — cannot open a task early, and neither can any other build of the code."))

    h.append("<h2><small>Step 4</small>Money</h2>")
    h.append(f"<p>Sponsors put {pool:.2f} NEAR into the season's pool. Each path out is one rule: <code>redeem</code> pays "
             "<code>balance × pool / supply</code> (both fixed at close) and burns the tokens; with no record, <code>reclaim</code> "
             "returns each sponsor's own deposit; <code>refund_deposit</code> returns every entry's deposit, whatever its duel did.</p>")
    words = {"sponsor": "put into the pool", "redeem": "redeemed all its progress tokens for its share of the pool", "reclaim": "took back its sponsorship",
             "refund_deposit": "took back its entry deposit", "enter": "entered a model", "close": "closed the season"}
    rows = [f"<tr><td>{acct_link(t['signer'])}</td><td><code>{e(t['method'])}</code> — {words[t['method']]}"
            f"{(': ' + e(t['args'].get('model', ''))) if t['method'] == 'enter' else ''}</td><td>{tx_link(t['hash'])}</td></tr>"
            for t in log if t["method"] in words and t["status"] == "ok"]
    h.append("<table><tr><th>Account</th><th>Call</th><th>Transaction</th></tr>" + "".join(rows) + "</table>")
    h.append(proof(["money"], "Tokens were minted only for the confirmed gain, and the payout was exactly the one rule."))

    h.append("<h2><small>Design</small>Why each choice</h2><table><tr><th>Choice</th><th>Why</th></tr>" + "".join(
        f"<tr><td><b>{a}</b></td><td>{b}</td></tr>" for a, b in CHOICES) + "</table>")

    h.append("<h2><small>Next</small>Weights, not just models</h2>"
             "<p>This season's duels run between NEAR AI's catalog models, because its confidential inference serves its own catalog only. "
             "The arena is written for Delta League's format (<code>delta.submission.v1</code>, a version named by the hash of its content). "
             "One step is missing: if NEAR AI serves an uploaded weight delta under its content hash, the response signature it already makes — "
             "<code>model_id:sha256(request):sha256(response)</code> — binds every answer to exact weights. Then a company can put its own "
             "kind of problem and its own prize behind a season, and anyone who improves a model at it gets paid — with nobody, the company "
             "included, able to leak or tune the test.</p>")

    h.append("<h2><small>Trust</small>What you still trust</h2><ul>"
             "<li><b>The hardware</b>: Intel TDX and NVIDIA confidential GPUs, verified against Intel's and NVIDIA's services. Physical attacks on a host are outside Intel's model.</li>"
             "<li><b>OutLayer's workers and the NEAR MPC network's key derivation</b> (workers in TDX, their measurements approved on NEAR) for giving a build's keys only to that build and reporting a run's caller truthfully.</li>"
             "<li><b>NEAR AI's deployment</b> for which weights stand behind a model name: a quote attests the signing key and the software stack, not a hash of the weights.</li>"
             "</ul><p>Not the operator, not the organiser, and not this page.</p>")

    if verify_out and os.path.exists(verify_out):
        h.append("<h2><small>Terminal</small>The same checks, from a terminal</h2>")
        h.append(f"<pre>$ cargo run --release -p arena-verify -- --arena {e(acct)}\n\n{e(open(verify_out).read())}</pre>")

    shots = sorted(f for f in os.listdir(os.path.join(WEB, "shots")) if f[:2] in ("01", "02", "03", "04")) if os.path.isdir(os.path.join(WEB, "shots")) else []
    if shots:
        h.append("<h2><small>On chain</small>What NEAR itself answers</h2><p>Raw answers of NEAR RPC for this season, rendered as they came.</p>")
        for f in shots:
            caption = os.path.splitext(f)[0].split("-", 1)[-1].replace("-", " ")
            h.append(f"<figure class=shot><img src='shots/{e(f)}' alt='{e(caption)}' loading=lazy><figcaption>{e(caption)}</figcaption></figure>")

    h.append("<footer>Attested Arena · built at NEAR HACKATHON, November 2026 · "
             f"<a href='{REPO}'>source</a> · our other NEAR HACKATHON project: "
             "<a href='https://github.com/zavodil/near-attested-ai'>Provable AI Compliance</a></footer>")
    h.append("</main><script type=module src='verify/verify.js'></script></body></html>")

    page = "\n".join(h)
    parts = page.split("<h2>")
    out = parts[0]
    for part in parts[1:]:
        title = re.sub(r"<small>.*?</small>", "", part.split("</h2>", 1)[0])
        slug = re.sub(r"[^a-z0-9]+", "-", title.lower()).strip("-")
        out += f"</section><section id={slug}><h2>" + part
    out = out.replace("</section>", "", 1).replace("</main>", "</section></main>", 1)
    open(os.path.join(WEB, "index.html"), "w").write(out)
    print("wrote web/index.html and web/evidence/")


if __name__ == "__main__":
    main()
