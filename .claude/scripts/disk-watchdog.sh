#!/bin/bash
# Keep a cloud session's disk allowance from filling while agents build.
# Every 30s, in the shared build dir (<main checkout>/target):
#   <8 GB free → drop the coverage + release trees and artifacts >45 min old
#   <6 GB free → drop incremental caches
#   <3 GB free → drop the whole debug build (in-flight builds just rebuild)
# Usage: .claude/scripts/disk-watchdog.sh [seconds=7000] (run in background;
# re-arm when it exits). Actions are logged to $DISK_WATCHDOG_LOG.
repo=$(dirname "$(git rev-parse --path-format=absolute --git-common-dir)")
log=${DISK_WATCHDOG_LOG:-/tmp/curio-disk-watchdog.log}
end=$((SECONDS + ${1:-7000}))
free_gb() { df --output=avail -BG / | tail -1 | tr -dc '0-9'; }
while [ $SECONDS -lt $end ]; do
  f=$(free_gb)
  if [ "$f" -lt 8 ]; then
    rm -rf $repo/target/llvm-cov-target $repo/target/release 2>/dev/null
    find $repo/target/debug/deps $repo/target/debug/build \
      -mindepth 1 -maxdepth 1 -mmin +45 -exec rm -rf {} + 2>/dev/null
    echo "$(date -u +%FT%TZ) free=${f}G -> dropped cov/release + artifacts >45min old; now $(free_gb)G" >>"$log"
  fi
  f=$(free_gb)
  if [ "$f" -lt 6 ]; then
    rm -rf $repo/target/debug/incremental $repo/.claude/worktrees/*/target/debug/incremental 2>/dev/null
    echo "$(date -u +%FT%TZ) free=${f}G -> cleared incremental; now $(free_gb)G" >>"$log"
  fi
  f=$(free_gb)
  if [ "$f" -lt 3 ]; then
    rm -rf $repo/target/debug $repo/.claude/worktrees/*/target 2>/dev/null
    echo "$(date -u +%FT%TZ) free=${f}G -> cleared shared debug build; now $(free_gb)G" >>"$log"
  fi
  sleep 30
done
echo "$(date -u +%FT%TZ) watchdog exiting (free=$(free_gb)G)" >>"$log"
