#!/bin/sh
# Start an isolated Dingo devnet, verify its API, submit a faucet transaction,
# verify the resulting UTxO through Blockfrost, and always tear down.
set -eu

devnet_dir=$(CDPATH='' cd "$(dirname "$0")/.." && pwd)
cd "$devnet_dir"

if ! command -v docker >/dev/null 2>&1 || ! docker info >/dev/null 2>&1; then
  if [ -n "${CARDANO_INIT_REQUIRE_DEVNET:-}" ]; then
    echo "Dingo devnet required, but Docker is unavailable." >&2
    exit 1
  fi
  echo "Docker unavailable; skipping the Dingo devnet integration test."
  exit 0
fi
if ! docker compose version >/dev/null 2>&1; then
  echo "Dingo devnet requires Docker Compose v2." >&2
  exit 1
fi

port=${DINGO_DEVNET_BLOCKFROST_PORT:-3000}
case "$port" in
  ''|*[!0-9]*)
    echo "DINGO_DEVNET_BLOCKFROST_PORT must be numeric." >&2
    exit 1
    ;;
esac
if [ "$port" -lt 1 ] || [ "$port" -gt 65535 ]; then
  echo "DINGO_DEVNET_BLOCKFROST_PORT must be between 1 and 65535." >&2
  exit 1
fi

COMPOSE_PROJECT_NAME="cardano-init-dingo-test-$$"
export COMPOSE_PROJECT_NAME DINGO_DEVNET_BLOCKFROST_PORT="$port"

teardown() {
  status=$?
  trap - 0 1 2 15
  sh scripts/compose.sh down --volumes --remove-orphans >/dev/null 2>&1 || true
  exit "$status"
}
trap teardown 0 1 2 15

sh scripts/compose.sh up --detach

block=
attempt=0
while [ "$attempt" -lt 60 ]; do
  block=$(sh scripts/compose.sh exec -T dingo \
    wget -qO- http://127.0.0.1:3000/api/v0/blocks/latest 2>/dev/null || true)
  case "$block" in
    *'"hash":"'*'"slot":'*) break ;;
  esac
  attempt=$((attempt + 1))
  sleep 2
done
case "$block" in
  *'"hash":"'*'"slot":'*) ;;
  *)
    echo "Dingo Blockfrost API did not become ready." >&2
    sh scripts/compose.sh logs --tail 80 dingo >&2
    exit 1
    ;;
esac
echo "Dingo Blockfrost API is serving blocks."

params=$(sh scripts/compose.sh exec -T dingo \
  wget -qO- http://127.0.0.1:3000/api/v0/epochs/latest/parameters)
case "$params" in
  *'"protocol_major_ver":'*) ;;
  *)
    echo "Dingo Blockfrost API returned malformed protocol parameters." >&2
    exit 1
    ;;
esac

target_id=$$
sh scripts/compose.sh exec -T dingo \
  cardano-cli address key-gen \
  --verification-key-file "/tmp/cardano-init-target-$target_id.vkey" \
  --signing-key-file "/tmp/cardano-init-target-$target_id.skey"
target_address=$(sh scripts/compose.sh exec -T dingo \
  cardano-cli address build \
  --payment-verification-key-file "/tmp/cardano-init-target-$target_id.vkey" \
  --testnet-magic 42)

sh scripts/fund.sh "$target_address" 100000000

utxos=
attempt=0
while [ "$attempt" -lt 30 ]; do
  utxos=$(sh scripts/compose.sh exec -T dingo \
    wget -qO- "http://127.0.0.1:3000/api/v0/addresses/$target_address/utxos" 2>/dev/null || true)
  case "$utxos" in
    *'"unit":"lovelace","quantity":"100000000"'*) break ;;
  esac
  attempt=$((attempt + 1))
  sleep 1
done
case "$utxos" in
  *'"unit":"lovelace","quantity":"100000000"'*) ;;
  *)
    echo "Submitted faucet transaction was not indexed." >&2
    exit 1
    ;;
esac

echo "Dingo faucet transaction is indexed by the Blockfrost API."

# If an off-chain component is present, run its suite against this devnet too,
# the way the Yaci devnet test does. The faucet round trip above only proves
# plain value transfer; it never calls the provider's script-evaluation
# endpoint, which is why a devnet that could not build a single script
# transaction still passed this test. Running the real mint/lock/redeem
# round-trip here is what keeps that class of gap visible.
#
# Generic — keyed on the off-chain role's directory, not on any specific tool.
if [ -f ../off-chain/Justfile ]; then
  echo "Running off-chain integration tests against the devnet ..."
  # Dingo has no admin topup endpoint, so the suite funds through the faucet
  # script instead. It resolves its own directory, so an absolute path works
  # from the off-chain component's working directory, and it inherits
  # COMPOSE_PROJECT_NAME so it targets this ephemeral devnet.
  INDEXER_URL="http://localhost:$port/api/v0/" \
  CARDANO_INIT_FUND_CMD="sh $devnet_dir/scripts/fund.sh" \
    just -f ../off-chain/Justfile test
fi

echo "Dingo devnet integration test passed."
