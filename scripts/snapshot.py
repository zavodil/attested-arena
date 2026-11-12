#!/usr/bin/env python3
"""Saves, next to the report, every NEAR answer the in-browser check reads.

    scripts/snapshot.py <season>

The page checks this copy the moment it opens, and offers to read the same
answers live from NEAR and check again. File names follow
`web/verify/verify.js`: `<method>.json` for a view without arguments,
`<method>-<first 16 hex of sha256(args JSON)>.json` otherwise, `tx-<hash>.json`
for a transaction's status and `access-keys.json` for the arena's keys.
`meta.json` records the block the views were read at.
"""

import base64
import datetime
import hashlib
import json
import os
import sys

sys.path.insert(0, os.path.dirname(__file__))
import season as s  # noqa: E402
import tx  # noqa: E402

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
WEB = os.path.join(ROOT, "web")
OUT = os.path.join(WEB, "evidence", "snapshot")


def args_json(args):
    return json.dumps(args, separators=(",", ":"))


def call(url, method, params):
    r = tx.rpc(url, method, params)
    if "result" not in r:
        raise SystemExit(f"{method} failed: {json.dumps(r.get('error'))[:300]}")
    return r["result"]


def main():
    season = sys.argv[1]
    acct = s.arena(season)
    url = tx.rpc_url()
    block = call(url, "block", {"finality": "final"})["header"]
    at = {"block_id": block["height"]}

    def view(method, args=None):
        a = args_json(args or {})
        r = call(url, "query", {"request_type": "call_function", **at, "account_id": acct, "method_name": method,
                                "args_base64": base64.b64encode(a.encode()).decode()})
        value = json.loads(bytes(r["result"]))
        name = f"{method}.json" if a == "{}" else f"{method}-{hashlib.sha256(a.encode()).hexdigest()[:16]}.json"
        json.dump(value, open(os.path.join(OUT, name), "w"))
        return value

    os.makedirs(OUT, exist_ok=True)
    st = view("get_season")
    view("get_leaves", {"from": 0, "limit": 200})
    view("get_batches", {"from": 0, "limit": 50})
    portions = view("get_portions", {"from": 0, "limit": 50})
    view("get_duels", {"from": 0, "limit": 50})
    reveals = view("get_reveals", {"from": 0, "limit": 50})
    for i, p in enumerate(portions):
        if "Done" in p["state"]:
            view("get_evidence", {"key": f"p{i}"})
    for i, r in enumerate(reveals):
        if "Done" in r["state"]:
            view("get_evidence", {"key": f"r{i}"})

    keys = call(url, "query", {"request_type": "view_access_key_list", **at, "account_id": acct})
    json.dump(keys, open(os.path.join(OUT, "access-keys.json"), "w"))

    meta = json.load(open(os.path.join(WEB, "evidence", "state.json")))
    log = json.load(open(os.path.join(WEB, "evidence", "transactions.json")))
    experiments = json.load(open(os.path.join(WEB, "evidence", "experiments.json")))
    wanted = [(meta["identity_tx"], st["owner"]), (experiments["direct"]["tx"], st["owner"]), (experiments["other_build"]["tx"], st["owner"])]
    wanted += [(t["hash"], t["signer"]) for t in log if t["method"] == "redeem" and t["status"] == "ok"]
    for h, sender in wanted:
        r = call(url, "EXPERIMENTAL_tx_status", {"tx_hash": h, "sender_account_id": sender, "wait_until": "FINAL"})
        json.dump(r, open(os.path.join(OUT, f"tx-{h}.json"), "w"))

    saved = datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%d %H:%M UTC")
    json.dump({"block_height": block["height"], "block_hash": block["hash"], "saved_at": saved, "arena": acct},
              open(os.path.join(OUT, "meta.json"), "w"), indent=1)
    print(f"saved the arena at block {block['height']} ({saved}) and {len(wanted)} transactions to web/evidence/snapshot/")


if __name__ == "__main__":
    main()
