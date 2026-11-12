#!/usr/bin/env python3
"""Screenshots for the report and the README.

    scripts/shots.py <season>

Chain facts are shown as NEAR RPC answers, rendered as-is on a plain page
(explorers sit behind bot checks a headless browser cannot pass, and the raw
answer is what an engineer would check anyway). Report sections are cropped
from web/index.html. Playwright's CLI does the capture
(`npx playwright@1.44.1`, which matches the cached Chromium); PNGs go to
web/shots/ as `NN-caption.png`.
"""
import base64
import hashlib
import html
import json
import os
import subprocess
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import season as s  # noqa: E402
import tx  # noqa: E402

OUT = os.path.join(s.ROOT, "web", "shots")
TMP = os.path.join(s.ROOT, "scripts", ".state", "shots")
B58 = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz"

PAGE = """<!doctype html><meta charset=utf-8><style>
body{{margin:0;font:15px/1.5 -apple-system,Segoe UI,Inter,sans-serif;background:#fbfbfd;color:#16181d}}
main{{max-width:1180px;margin:0 auto;padding:28px}} h1{{font-size:22px;margin:0 0 4px}} .sub{{color:#5b6170;margin:0 0 16px}}
.call{{font:13px ui-monospace,Menlo,monospace;background:#eef3ff;border:1px solid #cdd9fb;border-radius:8px;padding:8px 12px;margin:14px 0 6px}}
pre{{background:#0f1115;color:#e6e6e6;padding:14px 16px;border-radius:10px;font:12.5px/1.45 ui-monospace,Menlo,monospace;white-space:pre-wrap;word-break:break-all;margin:0}}
.note{{background:#fff;border:1px solid #e3e6ec;border-radius:10px;padding:12px 16px;margin-top:14px}} b.ok{{color:#127a3a}}
</style><main><h1>{title}</h1><p class=sub>{sub}</p>{body}</main>"""


def b58(d):
    n, o = int.from_bytes(d, "big"), ""
    while n:
        n, r = divmod(n, 58)
        o = B58[r] + o
    return o


def capture(name, path, full=True, clip=None):
    target = os.path.join(OUT, f"{name}.png")
    cmd = ["npx", "-y", "playwright@1.44.1", "screenshot", "--viewport-size", "1280,860", "--wait-for-timeout", "600"]
    if full:
        cmd.append("--full-page")
    p = subprocess.run(cmd + ["file://" + path, target], capture_output=True, text=True, timeout=180)
    ok = os.path.exists(target)
    if ok and clip:
        from PIL import Image
        im = Image.open(target)
        top, height = clip
        im.crop((0, top, im.width, min(im.height, top + height))).save(target)
    print(("ok   " if ok else "FAIL ") + name, "" if ok else p.stderr[-300:])


def page(name, title, sub, blocks, note):
    body = "".join(f"<div class=call>{html.escape(c)}</div><pre>{html.escape(r)}</pre>" for c, r in blocks)
    body += f"<div class=note>{note}</div>"
    path = os.path.join(TMP, f"{name}.html")
    open(path, "w").write(PAGE.format(title=html.escape(title), sub=sub, body=body))
    capture(name, path)


def tx_logs(h, signer):
    r = tx.rpc(tx.rpc_url(), "EXPERIMENTAL_tx_status", {"tx_hash": h, "sender_account_id": signer, "wait_until": "FINAL"})
    res = r["result"]
    lines = []
    for o in res["receipts_outcome"]:
        out = o["outcome"]
        for log in out["logs"]:
            if len(log) > 700:
                log = log[:700] + " …"
            lines.append(f"[{out['executor_id']}] {log}")
    status = res["status"]
    value = None
    if "SuccessValue" in status and status["SuccessValue"]:
        value = base64.b64decode(status["SuccessValue"]).decode(errors="replace")
    return "\n".join(lines) + (f"\n\nreturn value: {value}" if value else "")


def main():
    season = sys.argv[1]
    acct = s.arena(season)
    os.makedirs(OUT, exist_ok=True)
    os.makedirs(TMP, exist_ok=True)
    for f in os.listdir(OUT):
        os.remove(os.path.join(OUT, f))
    url = tx.rpc_url()
    log = [json.loads(l) for l in open(os.path.join(s.ROOT, "scripts", ".state", f"{season}-txs.jsonl"))]
    first = lambda m: next(t for t in log if t["method"] == m and t["status"] == "ok")

    # 1. The arena: no access key at all, and the code NEAR runs is the reproducible build.
    acc = tx.rpc(url, "query", {"request_type": "view_account", "finality": "final", "account_id": acct})["result"]
    keys = tx.rpc(url, "query", {"request_type": "view_access_key_list", "finality": "final", "account_id": acct})["result"]
    built = os.path.join(s.ROOT, "dist", "attested_arena.wasm")
    local = b58(hashlib.sha256(open(built, "rb").read()).digest())
    match = "<b class=ok>equal</b>" if local == acc["code_hash"] else "<b>DIFFERENT</b>"
    page("01-the-arena-has-no-keys-and-runs-the-reproducible-build", f"NEAR RPC: {acct}",
         "What the network itself answers about the season account.",
         [(f'query view_access_key_list {{"account_id": "{acct}"}}', json.dumps({"keys": keys["keys"]}, indent=1)),
          (f'query view_account {{"account_id": "{acct}"}}', json.dumps({k: acc[k] for k in ("code_hash", "amount", "storage_usage")}, indent=1))],
         f"No key of any kind: nobody, the operator included, can deploy other code or sign as this account, so the rules that decide "
         f"which tasks a portion may open cannot change. <code>contract/build.sh</code> gives NEAR code hash <code>{local}</code> — "
         f"{match} to the code hash above.")

    # 2. The commitment.
    seal = first("seal")
    page("02-seal-fixes-the-merkle-root-before-any-duel", f"NEAR RPC: transaction {seal['hash']}",
         f"<code>{acct}.seal()</code>, signed by the operator before the first duel.",
         [(f"EXPERIMENTAL_tx_status {seal['hash']} — logs", tx_logs(seal["hash"], seal["signer"]))],
         "The contract computes the Merkle root over every leaf the judge signed. From here on no task can be added or replaced; "
         "<code>arena-verify</code> recomputes the same root from the leaves.")

    # 3. A judged portion: arena → OutLayer → judge in TDX → signed result back in the arena's callback.
    portions = s.view(acct, "get_portions", {"from": 0, "limit": 50})
    runs = [t for t in log if t["method"] == "run_portion" and t["status"] == "ok"]
    done = next(i for i, p in enumerate(portions) if "Done" in p["state"])
    run = runs[done]
    page("03-a-portion-the-arena-calls-the-judge-and-checks-its-signature", f"NEAR RPC: transaction {run['hash']}",
         f"<code>{acct}.run_portion()</code>: the arena calls OutLayer's <code>request_execution</code> with the judge's wasm URL; "
         "the judge runs in an Intel TDX enclave and its signed result returns in the arena's own callback.",
         [(f"EXPERIMENTAL_tx_status {run['hash']} — logs", tx_logs(run["hash"], run["signer"]))],
         "OutLayer's log names the run's cost and resources; the arena's callback accepted the result only because the judge's "
         "build-bound key signed it. The full result — tasks, answers, model signatures, Intel quotes — is stored with it.")

    # 4. Money out.
    redeem = first("redeem")
    page("04-progress-tokens-redeemed-for-the-sponsors-pool", f"NEAR RPC: transaction {redeem['hash']}",
         f"<code>{acct}.redeem()</code>, signed by {html.escape(redeem['signer'])}.",
         [(f"EXPERIMENTAL_tx_status {redeem['hash']} — logs", tx_logs(redeem["hash"], redeem["signer"])),
          (f'call_function {acct}.get_season — the season\'s money', json.dumps({k: s.view(acct, "get_season")[k] for k in ("closed_pool_yocto", "closed_supply", "token_supply")}, indent=1))],
         "One rule: <code>balance × pool / supply</code>, both fixed at close. The tokens are burned (NEP-141 <code>ft_burn</code> event) "
         "and the NEAR is transferred in the same transaction.")

    # 5–8. The report itself, section by section.
    node = os.path.join(s.ROOT, "scripts", ".state", "node")
    if not os.path.exists(os.path.join(node, "node_modules", "playwright")):
        subprocess.run(["npm", "install", "--prefix", node, "playwright@1.44.1"], capture_output=True, timeout=600)
    script = os.path.join(TMP, "sections.cjs")
    open(script, "w").write("""
const { chromium } = require(%s);
(async () => {
  const b = await chromium.launch(); const p = await b.newPage({ viewport: { width: 1280, height: 900 }, deviceScaleFactor: 2 });
  await p.goto('http://localhost:8765/', { waitUntil: 'networkidle' });
  await p.click('.verify-bar button.recheck');
  await p.waitForFunction(() => /checks passed|could not run/.test(document.querySelector('.verify-summary').textContent), null, { timeout: 240000 });
  const shots = [['05-what-this-season-proves-checked-in-the-browser', '#what-this-season-proves'],
                 ['06-architecture', 'section:has(.arch)'],
                 ['07-a-duel-and-an-opened-task', '#duels'], ['08-trying-to-get-around-it', '#trying-to-get-around-it'],
                 ['09-money-out-one-rule-per-path', '#money']];
  for (const [name, sel] of shots) {
    const el = p.locator(sel); if (await el.count() === 0) { console.log('missing ' + sel); continue; }
    await el.first().screenshot({ path: '%s/' + name + '.png' }); console.log('ok   ' + name);
  }
  await b.close();
})();
""" % (json.dumps(os.path.join(node, "node_modules", "playwright")), OUT))
    r = subprocess.run(["node", script], capture_output=True, text=True, timeout=300)
    print(r.stdout.strip() or r.stderr[-400:])
    from PIL import Image, ImageChops
    for f in sorted(os.listdir(OUT)):
        path = os.path.join(OUT, f)
        im = Image.open(path).convert("RGB")
        if f.startswith("07-"):
            im = im.crop((0, 0, im.width, min(im.height, 2600)))  # the first duel and its opened task
        bg = Image.new("RGB", im.size, im.getpixel((im.width - 1, im.height - 1)))
        box = ImageChops.difference(im, bg).getbbox()
        if box:
            im = im.crop((0, 0, im.width, min(im.height, box[3] + 24)))
        im.save(path)


if __name__ == "__main__":
    main()
