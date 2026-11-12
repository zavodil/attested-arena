#!/usr/bin/env python3
"""Send one function call to NEAR testnet and wait until every receipt it
started is final — including OutLayer's yield/resume, which outlives an RPC's
own wait.

    scripts/tx.py <signer> <receiver> <method> '<json args>' [--deposit NEAR] [--gas TGAS]

The transaction is signed by near CLI from ~/.near-credentials (legacy
keychain), sent with `send_tx`, and its hash computed locally, so a slow
execution never loses the hash. The final outcome is saved under
scripts/.state/tx/<hash>.json and summarised on stdout: status, logs, and
any receipt that failed.

RPC: a FastNEAR URL with the API key from FASTNEAR_API_KEY or near-cli's
config. The URL is never printed.
"""
import base64
import hashlib
import json
import os
import re
import subprocess
import sys
import time
import urllib.error
import urllib.request

STATE = os.path.join(os.path.dirname(os.path.abspath(__file__)), ".state", "tx")
B58 = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz"


def b58(data: bytes) -> str:
    n = int.from_bytes(data, "big")
    out = ""
    while n:
        n, r = divmod(n, 58)
        out = B58[r] + out
    return "1" * (len(data) - len(data.lstrip(b"\0"))) + out


def rpc_url() -> str:
    key = os.environ.get("FASTNEAR_API_KEY")
    if not key:
        for cfg in (os.path.expanduser("~/Library/Application Support/near-cli/config.toml"),
                    os.path.expanduser("~/.config/near-cli/config.toml")):
            try:
                m = re.search(r"rpc\.testnet\.fastnear\.com/?\?apiKey=([^\"'&\s]+)", open(cfg).read())
            except OSError:
                continue
            if m:
                key = m.group(1)
                break
    if not key:
        print("⚠ no FastNEAR API key: using the unkeyed RPC", file=sys.stderr)
        return "https://rpc.testnet.fastnear.com"
    return f"https://rpc.testnet.fastnear.com/?apiKey={key}"


def rpc(url: str, method: str, params) -> dict:
    """One JSON-RPC call. A transport failure comes back as an `error` member
    that names no URL: the URL carries the API key."""
    body = json.dumps({"jsonrpc": "2.0", "id": "arena", "method": method, "params": params}).encode()
    req = urllib.request.Request(url, body, {"content-type": "application/json"})
    try:
        with urllib.request.urlopen(req, timeout=60) as r:
            return json.load(r)
    except urllib.error.HTTPError as e:
        return {"error": {"transport": f"HTTP {e.code}"}}
    except (urllib.error.URLError, TimeoutError, OSError) as e:
        return {"error": {"transport": type(e).__name__}}


def sign(signer, receiver, method, args, deposit, gas) -> str:
    cmd = ["near", "--quiet", "contract", "call-function", "as-transaction", receiver, method, "json-args", args,
           "prepaid-gas", f"{gas} Tgas", "attached-deposit", f"{deposit} NEAR", "sign-as", signer,
           "network-config", "testnet", "sign-with-legacy-keychain", "display"]
    p = subprocess.run(cmd, capture_output=True, text=True)
    out = p.stdout + p.stderr
    m = re.search(r"Signed transaction \(serialized as base64\):\s*\n\s*([A-Za-z0-9+/=]+)", out)
    if not m:
        sys.exit("near CLI did not sign the transaction (check the signer's key in ~/.near-credentials)")
    return m.group(1)


def summarize(result: dict) -> dict:
    status = result.get("status", {})
    logs, failures = [], []
    for o in [result.get("transaction_outcome", {})] + result.get("receipts_outcome", []):
        out = o.get("outcome", {})
        logs += out.get("logs", [])
        st = out.get("status", {})
        if "Failure" in st:
            failures.append(st["Failure"])
    value = None
    if isinstance(status, dict) and "SuccessValue" in status:
        raw = base64.b64decode(status["SuccessValue"] or "")
        try:
            value = json.loads(raw) if raw else None
        except ValueError:
            value = raw.decode(errors="replace")
    return {"status": "ok" if "SuccessValue" in status else "failed", "value": value, "logs": logs, "failures": failures}


def send(signer, receiver, method, args, deposit="0", gas=300, wait_minutes=8):
    url = rpc_url()
    for attempt in range(4):
        signed = sign(signer, receiver, method, args, deposit, gas)
        raw = base64.b64decode(signed)
        tx_hash = b58(hashlib.sha256(raw[:-65]).digest())  # a SignedTransaction ends with its 65-byte signature
        r = rpc(url, "send_tx", {"signed_tx_base64": signed, "wait_until": "INCLUDED"})
        # near CLI can sign over a block hash the network already considers
        # stale; such a transaction never ran, so it is signed again.
        if "error" in r and "Expired" in json.dumps(r["error"]) and attempt < 3:
            time.sleep(2)
            continue
        break
    if "error" in r:
        sys.exit(f"send_tx refused the transaction: {json.dumps(r['error'])[:400]}")
    print(f"tx {tx_hash} ({signer} → {receiver}.{method})", file=sys.stderr)
    deadline = time.time() + wait_minutes * 60
    while time.time() < deadline:
        r = rpc(url, "EXPERIMENTAL_tx_status", {"tx_hash": tx_hash, "sender_account_id": signer, "wait_until": "FINAL"})
        if "result" in r:
            res = r["result"]
            pending = [o for o in res.get("receipts_outcome", []) if not o.get("outcome")]
            if res.get("final_execution_status") == "FINAL" and not pending:
                os.makedirs(STATE, exist_ok=True)
                with open(os.path.join(STATE, f"{tx_hash}.json"), "w") as f:
                    json.dump(res, f, indent=1)
                s = summarize(res)
                s["hash"] = tx_hash
                return s
        time.sleep(3)
    return {"status": "timeout", "hash": tx_hash, "value": None, "logs": [], "failures": []}


def main():
    a = sys.argv[1:]
    if len(a) < 4:
        sys.exit(__doc__)
    deposit = a[a.index("--deposit") + 1] if "--deposit" in a else "0"
    gas = int(a[a.index("--gas") + 1]) if "--gas" in a else 300
    s = send(a[0], a[1], a[2], a[3], deposit, gas)
    print(json.dumps(s, indent=1))
    sys.exit(0 if s["status"] == "ok" else 1)


if __name__ == "__main__":
    main()
