# DeepBook Sandbox Backend

Rust API server for executing DeepBook V3 swaps in a local Move VM using forked mainnet state. Builds orderbooks via MoveVM execution of DeepBook's `iter_orders` contract function at startup, then serves them over HTTP.

## Architecture

At startup, the server:

1. Loads pool object state from pre-cached JSONL files (exported from Snowflake at checkpoint 240M)
2. Fetches Move packages (DeepBook V3, Sui framework) via Sui gRPC
3. For each pool, creates a `sui-sandbox` `SimulationEnvironment`, loads all objects, and executes `iter_orders` via PTB to extract orders
4. Caches the resulting `SandboxOrderbook` (price levels with quantities) in memory
5. Serves orderbook data, MoveVM-backed swap quotes, and session-based trading over HTTP

The key insight is that DeepBook stores orders in a `BigVector` with BCS-encoded entries. Rather than manually parsing this, we let the Move VM decode orders correctly by calling the contract's own `iter_orders` function.

## Quick Start

```bash
# Set up environment
cp .env.example .env
# Edit .env with your SUI_GRPC_ENDPOINT and SUI_GRPC_API_KEY

# Build and run
cargo run
# Server starts on http://localhost:3001
```

Router compile/deploy is a required startup step. If router build or the local-VM router health check fails, backend startup exits with an error.
Use `../docs/RUNBOOK.md` for clean setup + troubleshooting playbook.

## Pure Local VM Flow (No HTTP Server)

Run the full DeepBook flow directly in-process:

```bash
cargo run --example full_deepbook_flow
```

This validates:

1. JSONL state loading for all pools
2. MoveVM orderbook build via `iter_orders`
3. Session creation and direct swap (`SUI -> USDC`)
4. Two-hop flow (`SUI -> USDC -> WAL`) with MoveVM router quoting

## API Endpoints

### Health

```
GET /health → "ok"
GET /api/startup-check → startup self-check JSON
```

### Sessions

```
POST /api/session           → Create session (returns session_id + initial balances)
GET  /api/session/:id       → Get session info + current balances
GET  /api/session/:id/history → Get swap history
POST /api/session/:id/reset → Reset balances to initial state
```

### Trading

```
POST /api/swap/quote        → Get quote (MoveVM PTB: pool views for direct, router for two-hop)
POST /api/swap              → Execute swap (requires session_id, updates balances)
GET  /api/balance/:id       → Get token balances for session
POST /api/faucet            → Fund session via local MoveVM faucet PTB (coin split + transfer)
GET  /api/debug/pool        → Read active debug pool/token config
GET  /api/debug/pools       → List created debug pools
POST /api/debug/pool        → Create+seed debug token/USDC pool (supports token metadata + seed params)
```

Notes:

- Sessions start with zero balances.
- Fund `DEEP` for routes that require fee budget during swap execution.

### Orderbook

```
GET /api/pools                        → List available pools
GET /api/orderbook?pool=sui_usdc      → Full orderbook snapshot
GET /api/orderbook/depth?pool=sui_usdc → Binance-style depth (bids/asks arrays)
GET /api/orderbook/stats?pool=sui_usdc → Pool statistics (mid, spread, depth)
```

## Project Structure

```
src/
├── main.rs                      # Server entry, MoveVM orderbook build at startup
├── lib.rs                       # Library crate root
├── api/
│   ├── mod.rs                   # Router, AppState (pool_registry, session_manager, orderbooks)
│   ├── session.rs               # Session CRUD endpoints
│   ├── balance.rs               # Balance queries + faucet
│   ├── swap.rs                  # MoveVM quote calls + swap execution
│   └── orderbook.rs             # Orderbook snapshot, depth, stats endpoints
├── sandbox/
│   ├── orderbook_builder.rs     # SimulationEnvironment + iter_orders PTB execution
│   ├── snowflake_bcs.rs         # JSON→BCS conversion for loading objects into MoveVM
│   ├── state_loader.rs          # Pool configs, JSONL file loading, BigVector discovery
│   └── swap_executor.rs         # Session management, balance tracking, swap execution
└── types/
    └── mod.rs                   # ApiError, ApiResult
```

## Data Files

Pre-cached pool state from Snowflake at checkpoint 240M:

| File | Pool | Objects | Description |
|------|------|---------|-------------|
| `data/sui_usdc_state_cp240M.jsonl` | SUI/USDC | 24 | 10 slices/side, 118 bids, 166 asks |
| `data/deep_usdc_state_cp240M.jsonl` | DEEP/USDC | 10 | 2 ask + 4 bid slices |
| `data/wal_usdc_state_cp240M.jsonl` | WAL/USDC | 6 | 2 ask + 1 bid slices |

To export fresh state, see `scripts/export_state_240m.py`.

## Examples

```bash
# Build and verify all three pool orderbooks via MoveVM
cargo run --example test_all_pools_240m

# Test single pool orderbook build
cargo run --example test_orderbook

# Run full local flow without starting the HTTP server
cargo run --example full_deepbook_flow
```

## Development

```bash
cargo build              # 0 warnings
cargo test               # 6 tests
cargo build --examples   # 0 warnings
RUST_LOG=debug cargo run # Verbose startup logging
```

## License

MIT
