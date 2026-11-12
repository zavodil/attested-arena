#!/usr/bin/env python3
"""Run an Attested Arena season on NEAR testnet, step by step.

    scripts/season.py deploy <season> <judge-sha256> [--near N]
    scripts/season.py generate <season> <family> <count> [--times K]
    scripts/season.py seal <season>
    scripts/season.py sponsor <season> <account> <NEAR>
    scripts/season.py enter <season> <account> <model>
    scripts/season.py duel <season>                  # start the next duel, run portions until it ends
    scripts/season.py close <season>
    scripts/season.py view <season> [method] [json]

<season> is the sub-account name under the operator: `season0` →
season0.attested-arena.testnet. Every transaction's outcome is saved under
scripts/.state/tx/. Keys come from ~/.near-credentials (legacy keychain).
"""
import json
import os
import subprocess
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import tx  # noqa: E402

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
OPERATOR = "attested-arena.testnet"
OUTLAYER = "outlayer.testnet"
CONTRACT_WASM = os.path.join(ROOT, "dist", "attested_arena.wasm")  # contract/build.sh: reproducible
RUN_DEPOSIT = "0.05"  # NEAR per judge run; OutLayer returns what it does not use

CONFIG = {
    "generator": "deepseek-ai/DeepSeek-V4.1-Flash-1017",
    "reader": "Qwen/Qwen3.8-27B",
    "base_model": "Qwen/Qwen3-VL-30B-A3B-Instruct",
    "models": ["Qwen/Qwen3-VL-30B-A3B-Instruct", "Qwen/Qwen3.6-35B-A3B-FP8", "z-ai/glm-5.3-flash"],
    "portion_size": 5,
    "max_portions": 6,
    # SPRT(p1 = 0.7, α = 0.05, β = 0.10), log ratios × 10^6: arena_core::sprt::Sprt::arena().
    "sprt": {"win": 336472, "loss": -510826, "accept": 2890372, "reject": -2251292},
    "entry_deposit": str(10**23),  # 0.1 NEAR
    "secrets_profile": "arena",
    "max_execution_seconds": 180,
    "max_instructions": 200_000_000_000,
}


def arena(season):
    return f"{season}.{OPERATOR}"


def judge_source(sha):
    return {"url": f"https://test.fastfs.io/{OPERATOR}/{OUTLAYER}/{sha}.wasm", "hash": sha, "build_target": "wasm32-wasip2"}


def call(signer, receiver, method, args, deposit="0", gas=300):
    r = tx.send(signer, receiver, method, json.dumps(args), deposit, gas)
    season = receiver.split(".")[0]
    with open(os.path.join(ROOT, "scripts", ".state", f"{season}-txs.jsonl"), "a") as f:
        f.write(json.dumps({"method": method, "signer": signer, "args": args, "hash": r["hash"], "status": r["status"],
                            "time": int(time.time())}) + "\n")
    for log in r["logs"]:
        if not log.startswith("EVENT_JSON") and not log.startswith("[[yNEAR"):
            print("   log:", log[:400])
    for f in r["failures"]:
        print("   failure:", json.dumps(f)[:600])
    print(f"   {method}: {r['status']}  (tx {r['hash']})")
    return r


def view(account, method, args=None):
    url = tx.rpc_url()
    import base64
    r = tx.rpc(url, "query", {"request_type": "call_function", "finality": "final", "account_id": account,
                             "method_name": method, "args_base64": base64.b64encode(json.dumps(args or {}).encode()).decode()})
    if "error" in r or "result" not in r or "result" not in r["result"]:
        raise SystemExit(f"view {method} failed: {json.dumps(r.get('error') or r.get('result'))[:300]}")
    return json.loads(bytes(r["result"]["result"]))


def near(*args):
    p = subprocess.run(["near", "--quiet", *args], capture_output=True, text=True)
    out = p.stdout + p.stderr
    if p.returncode != 0:
        lines = [l for l in out.splitlines() if "private" not in l.lower() and "seed" not in l.lower()]
        raise SystemExit("near CLI: " + " | ".join(lines[-6:]))
    return out


def identity(sha):
    """The judge's key, as the build itself says it in an on-chain run."""
    args = {"source": {"WasmUrl": judge_source(sha)}, "input_data": json.dumps({"op": "identity"}),
            "resource_limits": {"max_instructions": 10_000_000_000, "max_memory_mb": 256, "max_execution_seconds": 60},
            "response_format": "Json"}
    r = tx.send(OPERATOR, OUTLAYER, "request_execution", json.dumps(args), "0.05", 300)
    out = r["value"]
    if not isinstance(out, dict) or "payload" not in out:
        raise SystemExit(f"identity run gave no result: {r}")
    key = json.loads(out["payload"])["judge_key"]
    print(f"   judge key {key}  (tx {r['hash']})")
    return key, r["hash"]


def deploy(season, sha, near_amount):
    acct = arena(season)
    print(f"== {acct}: judge build {sha}")
    key, id_tx = identity(sha)
    near("account", "create-account", "fund-myself", acct, f"{near_amount} NEAR", "autogenerate-new-keypair",
         "save-to-legacy-keychain", "sign-as", OPERATOR, "network-config", "testnet", "sign-with-legacy-keychain", "send")
    print(f"   created {acct} with {near_amount} NEAR")
    init = {"owner": OPERATOR, "outlayer": OUTLAYER,
            "judge": {"wasm_url": judge_source(sha)["url"], "wasm_sha256": sha, "key": key}, "config": CONFIG}
    near("contract", "deploy", acct, "use-file", CONTRACT_WASM, "with-init-call", "new", "json-args", json.dumps(init),
         "prepaid-gas", "100 Tgas", "attached-deposit", "0 NEAR", "network-config", "testnet", "sign-with-legacy-keychain", "send")
    print("   contract deployed and initialised")
    lock(acct)
    state = {"arena": acct, "judge_sha256": sha, "judge_key": key, "identity_tx": id_tx, "config": CONFIG}
    save(season, state)


def lock(acct):
    """Delete every full-access key: the code can no longer change."""
    keys = tx.rpc(tx.rpc_url(), "query", {"request_type": "view_access_key_list", "finality": "final", "account_id": acct})
    for k in keys["result"]["keys"]:
        if k["access_key"]["permission"] == "FullAccess":
            near("account", "delete-keys", acct, "public-keys", k["public_key"], "network-config", "testnet",
                 "sign-with-legacy-keychain", "send")
            print(f"   deleted full-access key {k['public_key'][:20]}… — {acct} is locked")


def state_path(season):
    return os.path.join(ROOT, "scripts", ".state", f"{season}.json")


def save(season, state):
    os.makedirs(os.path.dirname(state_path(season)), exist_ok=True)
    json.dump(state, open(state_path(season), "w"), indent=1)


def generate(season, family, count, times):
    acct = arena(season)
    for i in range(times):
        t0 = time.time()
        r = call(OPERATOR, acct, "generate", {"family": family, "count": count}, RUN_DEPOSIT)
        s = view(acct, "get_season")
        b = view(acct, "get_batches", {"from": s["batches"] - 1, "limit": 1})[0]
        st = b["state"]
        verdict = "admitted %d of %d %s" % (b["admitted"], b["generated"], dict(b["rejected"])) if "Done" in st else st
        print(f"   batch {s['batches'] - 1} ({family}): {verdict}; pool {s['pool_tasks']} tasks; {time.time() - t0:.0f}s")


def duel(season):
    acct = arena(season)
    s = view(acct, "get_season")
    if s["active_duel"] is None:
        call(OPERATOR, acct, "start_duel", {})
        s = view(acct, "get_season")
    d = s["active_duel"]
    while True:
        duel_state = view(acct, "get_duels", {"from": d, "limit": 1})[0]
        if duel_state["outcome"] is not None:
            print(f"   duel {d}: {duel_state['challenger_model']} vs {duel_state['champion_model']} → {duel_state['outcome']}")
            return
        t0 = time.time()
        r = call(OPERATOR, acct, "run_portion", {}, RUN_DEPOSIT)
        if r["failures"]:
            if "too few unused tasks" in json.dumps(r["failures"]):
                call(OPERATOR, acct, "end_duel", {})
                print("   the pool cannot fill another portion: the duel ends undecided")
                continue
            print("   the arena refused the portion; the duel stops here")
            return
        s = view(acct, "get_season")
        p = view(acct, "get_portions", {"from": s["portions"] - 1, "limit": 1})[0]
        res = p["results"]
        summary = f"champion {sum(c for c, _ in res)}/{len(res)}, challenger {sum(x for _, x in res)}/{len(res)}" if "Done" in p["state"] else p["state"]
        duel_state = view(acct, "get_duels", {"from": d, "limit": 1})[0]
        print(f"   portion {s['portions'] - 1} ({p['phase']}): {summary}; llr {duel_state['llr']}; {time.time() - t0:.0f}s")


def main():
    a = sys.argv[1:]
    if not a:
        sys.exit(__doc__)
    cmd = a[0]
    if cmd == "deploy":
        n = a[a.index("--near") + 1] if "--near" in a else "12"
        deploy(a[1], a[2], n)
    elif cmd == "generate":
        k = int(a[a.index("--times") + 1]) if "--times" in a else 1
        generate(a[1], a[2], int(a[3]), k)
    elif cmd == "seal":
        if call(OPERATOR, arena(a[1]), "seal", {})["status"] != "ok":
            sys.exit(1)
        print(json.dumps(view(arena(a[1]), "get_season"), indent=1)[:600])
    elif cmd == "sponsor":
        sys.exit(0 if call(a[2], arena(a[1]), "sponsor", {}, a[3])["status"] == "ok" else 1)
    elif cmd == "enter":
        sys.exit(0 if call(a[2], arena(a[1]), "enter", {"model": a[3]}, "0.1")["status"] == "ok" else 1)
    elif cmd == "duel":
        duel(a[1])
    elif cmd == "close":
        call(OPERATOR, arena(a[1]), "close", {})
    elif cmd == "view":
        method = a[2] if len(a) > 2 else "get_season"
        args = json.loads(a[3]) if len(a) > 3 else {}
        print(json.dumps(view(arena(a[1]), method, args), indent=1))
    else:
        sys.exit(__doc__)


if __name__ == "__main__":
    main()
