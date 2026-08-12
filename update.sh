#!/usr/bin/env bash
set -euo pipefail

# Cron-safe, non-interactive project update script for scope-studio-audio.
# - No prompts
# - Explicit logging
# - Deterministic failure exit codes

REPO_DIR="/home/hinata/hermes/gitrepos/rust/scope-studio-audio"
SCRIPT_NAME="$(basename "$0")"
LOG_DIR="$REPO_DIR/.cron"
LOG_FILE="$LOG_DIR/update.log"
LOCK_FILE="$LOG_DIR/update.lock"

mkdir -p "$LOG_DIR"

# Keep lock to avoid overlapping runs.
exec 9>"$LOCK_FILE"
if ! flock -n 9; then
  echo "[$(date -Iseconds)] $SCRIPT_NAME: another instance is already running" >>"$LOG_FILE"
  exit 0
fi

# Cron on this host gives a tiny environment; set what we need.
export PATH="/home/hinata/.cargo/bin:/usr/local/bin:/usr/bin:/bin"
export HOME="/home/hinata"
cd "$REPO_DIR"

log() {
  echo "[$(date -Iseconds)] $*" | tee -a "$LOG_FILE"
}

trap 'log "$SCRIPT_NAME: failed at line ${LINENO:-?}"; exit 1' ERR

log "starting update"

# Hard-fail when tracked files are dirty unless user explicitly stashes/amends.
tracked_changes="$(git status --porcelain --untracked-files=no)"
if [ -n "$tracked_changes" ]; then
  log "tracked files dirty; refusing auto-update"
  printf "%s\n" "$tracked_changes" | tee -a "$LOG_FILE"
  exit 10
fi

CURRENT_BRANCH="$(git rev-parse --abbrev-ref HEAD)"
log "branch: $CURRENT_BRANCH"

if [ "$CURRENT_BRANCH" != "main" ]; then
  log "not on main branch ($CURRENT_BRANCH), aborting"
  exit 11
fi

log "fetching origin"
git fetch --prune --tags origin

log "pulling updates"
git pull --ff-only origin main

if ! command -v cargo >/dev/null 2>&1; then
  log "cargo not found in PATH=$PATH"
  exit 12
fi

log "building docs check"
cargo check --bin doki --features audio

if command -v ./scripts/install.sh >/dev/null 2>&1; then
  log "running local install script"
  ./scripts/install.sh
fi

log "finished update"

echo "$(date -Iseconds) $SCRIPT_NAME: ok" >>"$LOG_FILE"