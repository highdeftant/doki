#!/usr/bin/env bash
set -euo pipefail

REPO_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BIN_NAME="doki"
FEATURES="audio"
LOCKED="--locked"

usage() {
  cat <<'EOF'
Usage: ./scripts/install.sh [--help]

Install/upgrade doki from source.

Options:
  --path DIR    install from a custom local path (default: repo root)
  --help        show this help
EOF
}

while [[ ${#} -gt 0 ]]; do
  case "${1}" in
    --help|-h)
      usage
      exit 0
      ;;
    --path)
      if [[ $# -lt 2 ]]; then
        echo "--path requires a directory" >&2
        exit 2
      fi
      REPO_DIR="$(cd "${2}" && pwd)"
      shift 2
      ;;
    *)
      echo "Unknown option: ${1}" >&2
      usage
      exit 2
      ;;
  esac
done

if ! command -v cargo >/dev/null 2>&1; then
  echo "cargo not found. Install Rust first (https://rustup.rs)" >&2
  exit 1
fi

if ! [[ -f "${REPO_DIR}/Cargo.toml" ]]; then
  echo "No Cargo.toml found in ${REPO_DIR}" >&2
  exit 1
fi

cd "${REPO_DIR}"

# Keep install command simple across Linux/macOS:
echo "Installing ${BIN_NAME} via cargo from: ${REPO_DIR}"
cargo install --path . ${LOCKED} --features ${FEATURES} --bin ${BIN_NAME} --force

echo "Done."
echo "If you don't see doki in PATH, add this to your shell profile:"
echo "  export PATH=\"$HOME/.cargo/bin:\$PATH\""
