#!/usr/bin/env bash
# deploy.sh — Deploy MentorMinds contracts to Stellar testnet or mainnet.
set -euo pipefail

usage() {
  cat <<'USAGE'
Usage:
  ./scripts/deploy.sh [options]

Options:
  --network <testnet|mainnet>          Target network (default: testnet)
  --identity <name>                    Stellar CLI identity (default: default)
  --rpc-url <url>                      Override RPC URL
  --validation-cloud-key <key>         Validation Cloud key for mainnet RPC
  --fee-bps <u32>                      Escrow platform fee in basis points (default: 500)
  --auto-release-delay-secs <u64>      Escrow auto-release delay in seconds (default: 259200)
  --treasury <address>                 Treasury address (default: deployer address)
  --approved-tokens <json-array>       JSON list string for escrow initialization (default: [])
  --skip-build                         Skip cargo build
  --skip-fund                          Skip Friendbot funding on testnet
  --skip-init                          Skip contract initialization calls
  --skip-verify                        Skip post-deploy verification calls
  --force-redeploy                     Ignore existing IDs in deployed/<network>.json and redeploy
  --help                               Show this message

Examples:
  ./scripts/deploy.sh --network testnet --identity dev
  ./scripts/deploy.sh --network mainnet --identity prod --validation-cloud-key "$VALIDATION_CLOUD_KEY"
  ./scripts/deploy.sh --network testnet --fee-bps 300 --auto-release-delay-secs 172800
USAGE
}

NETWORK="testnet"
IDENTITY="default"
RPC_URL=""
VALIDATION_CLOUD_KEY_ARG="${VALIDATION_CLOUD_KEY:-}"
FEE_BPS="500"
AUTO_RELEASE_DELAY_SECS="259200"
TREASURY_OVERRIDE=""
APPROVED_TOKENS="[]"
SKIP_BUILD=0
SKIP_FUND=0
SKIP_INIT=0
SKIP_VERIFY=0
FORCE_REDEPLOY=0

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DEPLOYED_DIR="$REPO_ROOT/deployed"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --network)
      NETWORK="${2:-}"; shift 2 ;;
    --identity)
      IDENTITY="${2:-}"; shift 2 ;;
    --rpc-url)
      RPC_URL="${2:-}"; shift 2 ;;
    --validation-cloud-key)
      VALIDATION_CLOUD_KEY_ARG="${2:-}"; shift 2 ;;
    --fee-bps)
      FEE_BPS="${2:-}"; shift 2 ;;
    --auto-release-delay-secs)
      AUTO_RELEASE_DELAY_SECS="${2:-}"; shift 2 ;;
    --treasury)
      TREASURY_OVERRIDE="${2:-}"; shift 2 ;;
    --approved-tokens)
      APPROVED_TOKENS="${2:-}"; shift 2 ;;
    --skip-build)
      SKIP_BUILD=1; shift ;;
    --skip-fund)
      SKIP_FUND=1; shift ;;
    --skip-init)
      SKIP_INIT=1; shift ;;
    --skip-verify)
      SKIP_VERIFY=1; shift ;;
    --force-redeploy)
      FORCE_REDEPLOY=1; shift ;;
    --help|-h)
      usage; exit 0 ;;
    *)
      echo "Unknown arg: $1" >&2
      usage
      exit 1 ;;
  esac
done

if [[ "$NETWORK" != "testnet" && "$NETWORK" != "mainnet" ]]; then
  echo "ERROR: --network must be testnet or mainnet" >&2
  exit 1
fi

if ! [[ "$FEE_BPS" =~ ^[0-9]+$ ]]; then
  echo "ERROR: --fee-bps must be an integer" >&2
  exit 1
fi
if ! [[ "$AUTO_RELEASE_DELAY_SECS" =~ ^[0-9]+$ ]]; then
  echo "ERROR: --auto-release-delay-secs must be an integer" >&2
  exit 1
fi

CONFIG_FILE="$DEPLOYED_DIR/$NETWORK.json"
WASM_DIR="$REPO_ROOT/target/wasm32-unknown-unknown/release"
STELLAR_FLAGS=(--network "$NETWORK" --source "$IDENTITY")

log()  { echo "[deploy] $*"; }
ok()   { echo "  ✓ $*"; }
skip() { echo "  ↷ $* (already deployed)"; }

require_cmd() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "ERROR: missing required command: $1" >&2
    exit 1
  }
}

json_get() { jq -r ".$1 // empty" "$CONFIG_FILE" 2>/dev/null || true; }

json_set() {
  local key="$1" val="$2"
  local tmp
  tmp=$(mktemp)
  jq --arg k "$key" --arg v "$val" '.[$k] = $v' "$CONFIG_FILE" > "$tmp"
  mv "$tmp" "$CONFIG_FILE"
}

deploy_contract() {
  local name="$1" wasm="$2"
  local contract_id=""

  if [[ $FORCE_REDEPLOY -eq 0 ]]; then
    contract_id=$(json_get "$name")
  fi

  if [[ -n "$contract_id" ]]; then
    skip "$name → $contract_id"
    CONTRACT_ID="$contract_id"
    return
  fi

  log "Deploying $name ..."
  CONTRACT_ID=$(stellar contract deploy --wasm "$wasm" "${STELLAR_FLAGS[@]}" 2>/dev/null)
  json_set "$name" "$CONTRACT_ID"
  ok "$name → $CONTRACT_ID"
}

invoke() {
  local contract_id="$1"; shift
  stellar contract invoke --id "$contract_id" "${STELLAR_FLAGS[@]}" -- "$@" 2>&1 | grep -v '^$' || true
}

require_cmd stellar
require_cmd jq
require_cmd cargo
require_cmd curl

if [[ "$NETWORK" == "testnet" ]]; then
  if [[ -z "$RPC_URL" ]]; then
    RPC_URL="https://soroban-testnet.stellar.org:443"
  fi
  PASSPHRASE="Test SDF Network ; September 2015"
  FRIENDBOT_URL="https://friendbot.stellar.org"
else
  if [[ -z "$RPC_URL" ]]; then
    if [[ -z "$VALIDATION_CLOUD_KEY_ARG" ]]; then
      echo "ERROR: mainnet deploy requires --rpc-url or --validation-cloud-key" >&2
      exit 1
    fi
    RPC_URL="https://mainnet.stellar.validationcloud.io/v1/$VALIDATION_CLOUD_KEY_ARG"
  fi
  PASSPHRASE="Public Global Stellar Network ; September 2015"
  FRIENDBOT_URL=""
fi

mkdir -p "$DEPLOYED_DIR"
[[ -f "$CONFIG_FILE" ]] || echo '{}' > "$CONFIG_FILE"

stellar network add "$NETWORK" --rpc-url "$RPC_URL" --network-passphrase "$PASSPHRASE" 2>/dev/null || true

ADMIN=$(stellar keys address "$IDENTITY" 2>/dev/null)
if [[ -z "$ADMIN" ]]; then
  echo "ERROR: unable to resolve identity '$IDENTITY' via 'stellar keys address'" >&2
  exit 1
fi

TREASURY="$ADMIN"
if [[ -n "$TREASURY_OVERRIDE" ]]; then
  TREASURY="$TREASURY_OVERRIDE"
fi

if [[ "$NETWORK" == "testnet" && $SKIP_FUND -eq 0 ]]; then
  log "Funding $ADMIN via Friendbot ..."
  curl -sf "$FRIENDBOT_URL?addr=$ADMIN" -o /dev/null && ok "Funded" || log "Already funded (or Friendbot unavailable)"
fi

if [[ $SKIP_BUILD -eq 0 ]]; then
  log "Building contracts ..."
  (cd "$REPO_ROOT" && cargo build --target wasm32-unknown-unknown --release -q)
  ok "Build complete"
else
  log "Skipping build (--skip-build)"
fi

deploy_contract "escrow" "$WASM_DIR/mentorminds_escrow.wasm"
ESCROW_ID="$CONTRACT_ID"

deploy_contract "verification" "$WASM_DIR/mentorminds_verification.wasm"
VERIFICATION_ID="$CONTRACT_ID"

deploy_contract "mnt_token" "$WASM_DIR/mentorminds_mnt_token.wasm"
TOKEN_ID="$CONTRACT_ID"

if [[ $SKIP_INIT -eq 0 ]]; then
  log "Initializing contracts ..."

  invoke "$ESCROW_ID" initialize \
    --admin "$ADMIN" \
    --treasury "$TREASURY" \
    --fee_bps "$FEE_BPS" \
    --approved_tokens "$APPROVED_TOKENS" \
    --auto_release_delay_secs "$AUTO_RELEASE_DELAY_SECS" \
    2>&1 | grep -v "Already initialized" || true
  ok "escrow initialized"

  invoke "$VERIFICATION_ID" initialize --admin "$ADMIN" \
    2>&1 | grep -v "Already initialized" || true
  ok "verification initialized"

  invoke "$TOKEN_ID" initialize --admin "$ADMIN" \
    2>&1 | grep -v "Already initialized" || true
  ok "mnt_token initialized"
else
  log "Skipping initialization (--skip-init)"
fi

if [[ $SKIP_VERIFY -eq 0 ]]; then
  log "Verifying deployments ..."

  FEE=$(invoke "$ESCROW_ID" get_fee_bps)
  ok "escrow.get_fee_bps → $FEE"

  IS_VER=$(invoke "$VERIFICATION_ID" is_verified --mentor "$ADMIN")
  ok "verification.is_verified → $IS_VER"
else
  log "Skipping verification (--skip-verify)"
fi

json_set "network" "$NETWORK"
json_set "admin" "$ADMIN"
json_set "treasury" "$TREASURY"
json_set "fee_bps" "$FEE_BPS"
json_set "auto_release_delay_secs" "$AUTO_RELEASE_DELAY_SECS"
json_set "deployed_at" "$(date -u +%Y-%m-%dT%H:%M:%SZ)"

echo ""
echo "┌─────────────────────┬──────────────────────────────────────────────────────────┐"
printf "│ %-19s │ %-56s │\n" "Contract" "ID"
echo "├─────────────────────┼──────────────────────────────────────────────────────────┤"
for key in escrow verification mnt_token; do
  id=$(json_get "$key")
  printf "│ %-19s │ %-56s │\n" "$key" "$id"
done
echo "└─────────────────────┴──────────────────────────────────────────────────────────┘"
echo ""
echo "Config saved → $CONFIG_FILE"
