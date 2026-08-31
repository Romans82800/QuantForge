#!/usr/bin/env bash
# Install the fixed QuantForge build (equity partitions + OOS1 holdout) on macOS.
set -euo pipefail

REPO="${QF_REPO:-Romans82800/QuantForge}"
RUN_ID="${QF_RUN_ID:-33403642801}"
ARTIFACT="${QF_ARTIFACT:-QuantForge-macos}"

if ! command -v gh >/dev/null 2>&1; then
  echo "GitHub CLI (gh) is required. Install: brew install gh && gh auth login"
  exit 1
fi

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
cd "$TMP"

echo "Downloading $ARTIFACT from run $RUN_ID..."
gh run download "$RUN_ID" -R "$REPO" -n "$ARTIFACT"

if [[ -f QuantForge-macos.zip ]]; then
  unzip -q QuantForge-macos.zip
elif [[ -d QuantForge.app ]]; then
  :
else
  echo "Unexpected artifact layout:" >&2
  ls -la >&2
  exit 1
fi

osascript -e 'quit app "QuantForge"' 2>/dev/null || true
killall QuantForge 2>/dev/null || true
rm -rf /Applications/QuantForge.app
ditto QuantForge.app /Applications/QuantForge.app

echo "Installed QuantForge to /Applications/QuantForge.app"
open -a QuantForge
