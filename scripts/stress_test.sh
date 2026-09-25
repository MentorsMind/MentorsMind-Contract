#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
PYTHON=${PYTHON:-python3}

usage() {
  cat <<EOF
Usage: $0 [options]

Options:
  --endpoint URL           Base HTTP endpoint (required)
  --create-path PATH       POST path to create escrow (default: /escrow/create)
  --query-path PATH        GET path to query escrows (default: /escrow/list)
  --count N                Number of escrows to create (default: 10000)
  --concurrency N          Concurrent requests (default: 200)
  --payload TEMPLATE       JSON payload template (string)
  --output-dir DIR         Output directory (default: tests/stress/results)
EOF
}

if [ "$#" -eq 0 ]; then
  usage
  exit 1
fi

ARGS=()
while [[ $# -gt 0 ]]; do
  case "$1" in
    --endpoint) ARGS+=("--endpoint" "$2"); shift 2;;
    --create-path) ARGS+=("--create-path" "$2"); shift 2;;
    --query-path) ARGS+=("--query-path" "$2"); shift 2;;
    --count) ARGS+=("--count" "$2"); shift 2;;
    --concurrency) ARGS+=("--concurrency" "$2"); shift 2;;
    --payload) ARGS+=("--payload-template" "$2"); shift 2;;
    --output-dir) ARGS+=("--output-dir" "$2"); shift 2;;
    -h|--help) usage; exit 0;;
    *) echo "Unknown arg: $1"; usage; exit 1;;
  esac
done

${PYTHON} ${SCRIPT_DIR}/../tests/stress/runner.py "${ARGS[@]}"
