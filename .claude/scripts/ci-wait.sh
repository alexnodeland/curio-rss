#!/bin/bash
# Wait for a PR's CI on alexnodeland/curio-rss to finish (or a time cap).
# usage: ci-wait.sh <pr-number> [max-seconds=540]
# Prints: head sha, mergeable_state, check counts, then ALL_GREEN or
# FAILED: <name (job id)>; ... or TIMEOUT. Exit 0 always (read the output).
set -uo pipefail
pr=$1
max=${2:-540}
api=https://api.github.com/repos/alexnodeland/curio-rss
start=$SECONDS
while :; do
  pj=$(curl -sS "$api/pulls/$pr")
  sha=$(jq -r '.head.sha' <<<"$pj")
  state=$(jq -r '.mergeable_state' <<<"$pj")
  merged=$(jq -r '.merged' <<<"$pj")
  runs=$(curl -sS "$api/commits/$sha/check-runs?per_page=100")
  total=$(jq '.total_count // 0' <<<"$runs")
  pending=$(jq '[.check_runs[]? | select(.status != "completed")] | length' <<<"$runs")
  failed=$(jq -r '[.check_runs[]? | select(.status == "completed" and ((.conclusion == "success" or .conclusion == "skipped" or .conclusion == "neutral") | not)) | "\(.name) (job \(.id))"] | join("; ")' <<<"$runs")
  if [ "$total" -gt 0 ] && [ "$pending" -eq 0 ]; then break; fi
  if [ $((SECONDS - start)) -ge "$max" ]; then
    echo "TIMEOUT after ${max}s: checks=$total pending=$pending"
    break
  fi
  sleep 30
done
echo "pr=$pr head=$sha mergeable_state=$state merged=$merged checks=$total pending=$pending"
if [ -n "$failed" ]; then
  echo "FAILED: $failed"
elif [ "$total" -gt 0 ] && [ "$pending" -eq 0 ]; then
  echo "ALL_GREEN"
fi
