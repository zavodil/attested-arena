#!/bin/bash
# The whole demo season, end to end, as it ran on testnet:
#   scripts/run_season.sh <season> <judge-sha256> <probe-sha256>
# (after `scripts/season.py deploy <season> <judge-sha256>`).
set -u
S=$1; JUDGE=$2; PROBE=$3
cd "$(dirname "$0")/.."
py() { python3 -I scripts/season.py "$@" 2>&1 | grep -vE "^tx |Retrieving large output|Execution completed successfully"; }
for i in 1 2 3 4 5 6 7; do
    py generate "$S" seating 8
    if [ "$i" -le 6 ]; then py generate "$S" lines 8; fi
done
must() { python3 -I scripts/season.py "$@" 2>&1 | grep -vE "^tx "; [ "${PIPESTATUS[0]}" -eq 0 ] || { echo "STOP: $* failed"; exit 1; }; }
must seal "$S"
must sponsor "$S" arena-sponsor.testnet 5
must enter "$S" arena-bob.testnet z-ai/glm-5.3-flash          # a stronger model
must enter "$S" arena-alice.testnet Qwen/Qwen3.6-35B-A3B-FP8  # a weaker one
must enter "$S" arena-carol.testnet z-ai/glm-5.3-flash        # the champion's own model, entered again
py duel "$S"; py duel "$S"; py duel "$S"
python3 -I scripts/experiments.py "$S" "$PROBE" 2>&1 | grep -v "^tx "
py close "$S"
python3 -I - "$S" <<'PY'
import sys; sys.path.insert(0, "scripts"); import season as s
acct = s.arena(sys.argv[1]); st = s.view(acct, "get_season")
# Everything never opened: the unused remainder, and the tasks of failed portions.
unused = [i for p in s.view(acct, "get_portions", {"from": 0, "limit": st["portions"]}) if "Failed" in p["state"]
          for i in range(p["first_task"], p["first_task"] + p["count"])] + list(range(st["next_task"], st["pool_tasks"]))
for i in range(0, len(unused), 20):
    s.call(s.OPERATOR, acct, "reveal", {"tasks": unused[i:i + 20]}, s.RUN_DEPOSIT)
for who in ["arena-bob", "arena-alice", "arena-carol"]:
    if int(s.view(acct, "ft_balance_of", {"account_id": f"{who}.testnet"})) > 0:
        s.call(f"{who}.testnet", acct, "redeem", {})
for i, e in enumerate(s.view(acct, "get_entries", {"from": 0, "limit": 10})):
    s.call(e["account"], acct, "refund_deposit", {"entry": i})
PY
