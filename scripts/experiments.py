#!/usr/bin/env python3
"""Try to get around the seal, on testnet, and record what happened.

    scripts/experiments.py <season> <probe-sha256>

1. `direct`: the operator calls the judge's own build directly — not through
   the arena — asking it to open the arena's next unused task. The judge
   must refuse: only the locked arena may ask.
2. `other_build`: the operator runs another build (`probe/`) that declares the
   judge's exact `pool` key and tries the arena's next unused ciphertext with
   the judge's exact aad. The keystore derives keys from the build's sha256,
   so it must not open.

Results go to scripts/.state/experiments-<season>.json with the transaction
hashes; neither experiment ever prints a task.
"""
import json
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import season as s  # noqa: E402
import tx  # noqa: E402


def run(source, input_data, limits=None):
    args = {"source": {"WasmUrl": source}, "input_data": json.dumps(input_data),
            "resource_limits": limits or {"max_instructions": 10_000_000_000, "max_memory_mb": 256, "max_execution_seconds": 60},
            "response_format": "Json"}
    return tx.send(s.OPERATOR, s.OUTLAYER, "request_execution", json.dumps(args), "0.05", 300)


def main():
    season, probe_sha = sys.argv[1], sys.argv[2]
    acct = s.arena(season)
    st = s.view(acct, "get_season")
    state = json.load(open(s.state_path(season)))
    # A task that is sealed and was never opened: the next unused one, or —
    # when the pool is spent — one of a failed portion's, spent unopened.
    i = st["next_task"]
    if i >= st["pool_tasks"]:
        failed = [p for p in s.view(acct, "get_portions", {"from": 0, "limit": st["portions"]}) if "Failed" in p["state"]]
        if not failed:
            raise SystemExit("no sealed, unopened task is left")
        i = failed[0]["first_task"]
    leaf = s.view(acct, "get_leaves", {"from": i, "limit": 1})[0]
    ct = s.view(acct, "get_ciphertext", {"index": i})
    sealed = {"leaf": leaf, "ciphertext": ct}
    results = {"season": acct, "task_index": i, "leaf": leaf}

    print(f"== 1. the operator asks the judge directly for sealed task {i}, never opened")
    r = run(s.judge_source(state["judge_sha256"]), {"op": "reveal", "arena": acct, "tasks": [sealed]})
    print("   output:", json.dumps(r["value"])[:300])
    results["direct"] = {"tx": r["hash"], "output": r["value"]}

    print(f"== 2. another build, with the judge's own key declaration, tries the same ciphertext")
    r = run(s.judge_source(probe_sha) | {"url": f"https://test.fastfs.io/{s.OPERATOR}/{s.OUTLAYER}/{probe_sha}.wasm"},
            {"arena": acct, "leaf": leaf, "ciphertext": ct})
    print("   output:", json.dumps(r["value"])[:300])
    results["other_build"] = {"tx": r["hash"], "probe_sha256": probe_sha, "output": r["value"]}

    path = os.path.join(s.ROOT, "scripts", ".state", f"experiments-{season}.json")
    json.dump(results, open(path, "w"), indent=1)
    print("saved", path)


if __name__ == "__main__":
    main()
