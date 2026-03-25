# Service Marketplace Contract

A CosmWasm reverse-auction marketplace. Clients post **Asks** (requests for work), providers submit **Bids**, the client selects a winner and locks funds into escrow, and upon mutual completion the funds are released. An admin can resolve disputes off-chain and execute the resolution on-chain.

## Prerequisites

- [Rust](https://rustup.rs/) with `wasm32-unknown-unknown` target
- [Docker](https://www.docker.com/) (for optimized WASM builds)
- [`just`](https://github.com/casey/just) task runner
- [`xiond`](https://docs.burnt.com) CLI (for testnet deployment)

## Development

### Run tests
```sh
just test
```

### Lint
```sh
just lint
```

### Format
```sh
just fmt
just toml-format
```

### Generate schemas
```sh
just schema
```

## Build

Produces an optimized, reproducible WASM binary via the CosmWasm optimizer Docker image. Output lands in `artifacts/`.

```sh
just optimize
```

> Requires Docker running locally.

## Deploy to Testnet (Xion testnet-2)

The `upload.sh` script stores the contract and instantiates it in one step using the `testnet` key from your local keyring.

```sh
./upload.sh
```

It will:
1. Resolve the deployer address from the `testnet` key
2. Store `artifacts/service_marketplace.wasm` on-chain and extract the `code_id`
3. Instantiate the contract with `owner` set to the deployer address and `uxion` as the only allowed denom
4. Print the final contract address

Override the key with `FROM=mykey ./upload.sh`.

### Sample instantiate message

```json
{
  "owner": "xion1...",
  "allowed_denoms": ["uxion"]
}
```

Pass `"allowed_denoms": []` to accept any denom.

## Contract Overview

### State machine

```
[Open] ──SelectBid──► [InProgress] ──MarkComplete──► [WorkSubmitted] ──ConfirmCompletion──► [Completed]
  │                        │                                │
  └──CancelAsk──►[Cancelled]└──────────────────────────────┴──AdminCancel──► [Cancelled]
```

### Key messages

| Message | Caller | Description |
|---|---|---|
| `CreateAsk` | Client | Post a request for work with a budget hint and metadata URL |
| `SubmitBid` | Provider | Bid on an open ask |
| `SelectBid` | Client | Choose a winning bid and lock funds into escrow |
| `MarkComplete` | Provider | Signal work is done |
| `ConfirmCompletion` | Client | Release escrowed funds to provider |
| `CancelAsk` | Client | Cancel an ask that hasn't received a bid selection yet |
| `PartialDisbursement` | Client | Release part of the escrow early |
| `AdminCancel` | Owner | Cancel any active ask and refund escrowed funds |
| `AdminForceRelease` | Owner | Force-release all escrow to a recipient |
| `AdminResolve` | Owner | Split escrow across up to 5 recipients to resolve a dispute |
| `AdminSetAllowedDenoms` | Owner | Update the denom allowlist |
