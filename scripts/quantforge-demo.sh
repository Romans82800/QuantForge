#!/bin/sh
set -eu

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
PROJECT_DIR=$(CDPATH= cd -- "$SCRIPT_DIR/.." && pwd)
OUTPUT_DIR=${1:-"$PROJECT_DIR/demo-output"}
LAUNCH_APP=${2:-""}

if [ -e "$OUTPUT_DIR" ]; then
  echo "Refusing to replace existing demo output: $OUTPUT_DIR" >&2
  echo "Pass a new output directory as the first argument." >&2
  exit 1
fi

mkdir -p "$OUTPUT_DIR"

DATA="$PROJECT_DIR/fixtures/EURUSD_M15_sample.tsv"
METADATA="$PROJECT_DIR/fixtures/EURUSD_M15_sample.metadata.csv"
BROKER="$PROJECT_DIR/fixtures/EURUSD_fixture_broker.json"
BANK="$OUTPUT_DIR/databank.json"

echo "[1/3] Validating the MT5 sample history"
cargo run --quiet --manifest-path "$PROJECT_DIR/Cargo.toml" -p quantforge-cli -- \
  data-quality "$DATA" \
  --metadata "$METADATA" \
  --out "$OUTPUT_DIR/data-quality.json"

echo "[2/3] Illuminating a deterministic four-family databank"
cargo run --quiet --manifest-path "$PROJECT_DIR/Cargo.toml" -p quantforge-cli -- \
  evolve "$DATA" \
  --metadata "$METADATA" \
  --broker "$BROKER" \
  --databank "$BANK" \
  --initial 120 \
  --generations 1 \
  --batch 40 \
  --correlation 0.88 \
  --seed 42 \
  --minimum-trades 0 \
  --maximum-drawdown-percent 100 \
  --minimum-return-percent=-100 \
  --minimum-profit-factor 0 \
  --commission-per-lot-round-turn 7 \
  --slippage-points-per-side 1 \
  --fallback-spread-points 8

echo "[3/3] Continuing the same immutable search recipe"
cargo run --quiet --manifest-path "$PROJECT_DIR/Cargo.toml" -p quantforge-cli -- \
  evolve "$DATA" \
  --metadata "$METADATA" \
  --broker "$BROKER" \
  --databank "$BANK" \
  --continue \
  --generations 1

echo
echo "QuantForge demo databank is ready:"
echo "  $BANK"
echo
echo "Open QuantForge, choose 'Open databank', and select that file."

if [ "$LAUNCH_APP" = "--launch" ]; then
  APP="$PROJECT_DIR/target/debug/bundle/macos/QuantForge.app"
  if [ "$(uname -s)" != "Darwin" ]; then
    echo "--launch is currently available only for the macOS bundle." >&2
    exit 1
  fi
  if [ ! -d "$APP" ]; then
    echo "The native app bundle has not been built: $APP" >&2
    echo "Run 'cd apps/desktop && pnpm tauri build --debug' first." >&2
    exit 1
  fi
  open "$APP"
fi
