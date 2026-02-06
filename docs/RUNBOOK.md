# DeepBook Sandbox Runbook

This runbook is for clean local setup and reproducible backend verification using VM-native execution paths.

## 1. Clean Setup

```bash
git clone --recurse-submodules https://github.com/Evan-Kim2028/sandbox-deepbook.git
cd sandbox-deepbook
git submodule update --init --recursive
```

Backend prerequisites:

- Rust toolchain (`rustup`, stable)
- `sui` CLI on `PATH`
- Network access to Sui mainnet gRPC (`https://archive.mainnet.sui.io:443`)

Start backend:

```bash
cd backend
cargo run
```

Start frontend:

```bash
cd ../frontend
npm install
npm run dev
```

## 2. Required Startup Verification

The backend now exposes an explicit startup self-check:

```bash
curl -s http://localhost:3001/api/startup-check | jq
```

Expected:

- `ok: true`
- `router_package_deployed: true`
- `router_health_check_passed: true`
- required shared objects marked `present: true`
- reserve coins (`SUI/USDC/WAL/DEEP`) marked `present: true` with non-zero `value`

If this endpoint is not healthy, do not proceed to frontend integration tests.

## 3. End-to-End Backend Flow (VM-only)

Create session:

```bash
SESSION_ID=$(curl -s -X POST http://localhost:3001/api/session \
  -H "Content-Type: application/json" \
  -d '{}' | jq -r '.session_id')
```

Create custom token/pool:

```bash
curl -s -X POST http://localhost:3001/api/debug/pool \
  -H "Content-Type: application/json" \
  -d '{
    "token_symbol":"ALPHA",
    "token_name":"Alpha Sandbox Token",
    "token_description":"VM-created token for debug pool flow",
    "token_icon_url":"",
    "bid_price":900000,
    "ask_price":1100000,
    "bid_quantity":100000000000,
    "ask_quantity":100000000000,
    "base_liquidity":200000000000,
    "quote_liquidity":200000000
  }' | jq
```

List custom pools:

```bash
curl -s http://localhost:3001/api/debug/pools | jq
```

Fund session through VM faucet PTBs:

```bash
curl -s -X POST http://localhost:3001/api/faucet \
  -H "Content-Type: application/json" \
  -d "{\"session_id\":\"$SESSION_ID\",\"token\":\"sui\",\"amount\":\"10000000000\"}" | jq

curl -s -X POST http://localhost:3001/api/faucet \
  -H "Content-Type: application/json" \
  -d "{\"session_id\":\"$SESSION_ID\",\"token\":\"deep\",\"amount\":\"10000000\"}" | jq

curl -s -X POST http://localhost:3001/api/faucet \
  -H "Content-Type: application/json" \
  -d "{\"session_id\":\"$SESSION_ID\",\"token\":\"alpha\",\"amount\":\"100000000000\"}" | jq
```

Execute router-backed swap:

```bash
curl -s -X POST http://localhost:3001/api/swap \
  -H "Content-Type: application/json" \
  -d "{\"session_id\":\"$SESSION_ID\",\"from_token\":\"sui\",\"to_token\":\"alpha\",\"amount\":\"1000000000\"}" | jq
```

## 4. Troubleshooting

### `startup-check.ok = false`

- Check `errors` in `/api/startup-check` first.
- If shared objects are missing:
  - verify outbound connectivity to mainnet gRPC.
  - restart backend after connectivity is restored.
- If reserve bootstrap is missing:
  - ensure gRPC checkpoint/object fetches are available.
  - restart backend (bootstrap runs at startup).
- If router health check fails:
  - run `sui move build --force` in `contracts/router`.
  - confirm `deepbookv3` submodule is present and up to date.

### `POST /api/debug/pool` fails with config mismatch

- Current runtime supports one custom debug token/pool per backend process.
- Restart backend to apply a different token/pool config.
- Token decimals are hard-coded to 9 for the current debug token flow.

### Swap aborts with DeepBook `place_order_int` / abort code

- Ensure session has DEEP balance funded via `/api/faucet`.
- Some swap paths consume DEEP fee budget in VM execution.

### Frontend appears to lose session

- Session ID is persisted in browser local storage.
- If backend restarted, the old session may be invalidated; frontend auto-creates a fresh session.
