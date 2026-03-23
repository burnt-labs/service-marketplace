#!/usr/bin/env bash
set -euo pipefail

# ---------------------------------------------------------------------------
# Configuration — update these as needed
# ---------------------------------------------------------------------------
NODE="https://rpc.xion-testnet-2.burnt.com:443"
CHAIN_ID="xion-testnet-2"
FROM="${FROM:-testnet}"
GAS_PRICES="0.01uxion"
WASM="artifacts/service_marketplace.wasm"

# ---------------------------------------------------------------------------
# Validation
# ---------------------------------------------------------------------------

if [[ ! -f "$WASM" ]]; then
  echo "Error: $WASM not found — run 'just optimize' first"
  exit 1
fi

OWNER=$(xiond keys show "$FROM" -a)
echo "Owner address: $OWNER"

# ---------------------------------------------------------------------------
# Store
# ---------------------------------------------------------------------------
echo "Uploading $WASM to $CHAIN_ID..."

STORE_RES=$(xiond tx wasm store "$WASM" \
  --from "$FROM" \
  --node "$NODE" \
  --chain-id "$CHAIN_ID" \
  --gas-prices "$GAS_PRICES" \
  --gas auto \
  --gas-adjustment 1.5 \
  --output json \
  --yes)

echo "$STORE_RES" | jq .

TXHASH=$(echo "$STORE_RES" | jq -r '.txhash')
echo ""
echo "Tx hash: $TXHASH"
echo "Waiting for tx to be included..."
sleep 6

# ---------------------------------------------------------------------------
# Fetch code_id from tx result
# ---------------------------------------------------------------------------
TX_RES=$(xiond query tx "$TXHASH" \
  --node "$NODE" \
  --output json)

CODE_ID=$(echo "$TX_RES" | jq -r '
  .events[]
  | select(.type == "store_code")
  | .attributes[]
  | select(.key == "code_id")
  | .value
')

echo ""
echo "✅ Contract stored — code_id: $CODE_ID"

# ---------------------------------------------------------------------------
# Instantiate
# ---------------------------------------------------------------------------
echo "Instantiating contract..."

INIT_MSG="{\"owner\":\"$OWNER\",\"allowed_denoms\":[\"uxion\"]}"

INIT_RES=$(xiond tx wasm instantiate "$CODE_ID" "$INIT_MSG" \
  --label "service-marketplace" \
  --no-admin \
  --from "$FROM" \
  --node "$NODE" \
  --chain-id "$CHAIN_ID" \
  --gas-prices "$GAS_PRICES" \
  --gas auto \
  --gas-adjustment 1.5 \
  --output json \
  --yes)

echo "$INIT_RES" | jq .

INIT_TXHASH=$(echo "$INIT_RES" | jq -r '.txhash')
echo ""
echo "Tx hash: $INIT_TXHASH"
echo "Waiting for tx to be included..."
sleep 6

INIT_TX_RES=$(xiond query tx "$INIT_TXHASH" \
  --node "$NODE" \
  --output json)

CONTRACT_ADDR=$(echo "$INIT_TX_RES" | jq -r '
  .events[]
  | select(.type == "instantiate")
  | .attributes[]
  | select(.key == "_contract_address")
  | .value
')

echo ""
echo "✅ Contract instantiated — address: $CONTRACT_ADDR"
