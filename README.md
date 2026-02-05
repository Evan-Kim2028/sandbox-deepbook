# DeepBook Sandbox

Interactive web application for experiencing Sui's DeepBook V3 protocol in a forked mainnet environment. Execute real swaps against production orderbook state with zero risk, no testnet tokens, and no gas costs.

## How It Works

The backend loads real DeepBook V3 pool state from a Snowflake checkpoint (240M) and reconstructs live orderbooks using the **Move VM** via `sui-sandbox`. This means the order decoding, price extraction, and orderbook iteration are done by executing the actual DeepBook Move contract (`iter_orders`), not by manually parsing BCS bytes.

```
                                   Startup
                                      │
         Snowflake (checkpoint 240M)  │  Sui gRPC (packages only)
         ┌──────────────┐            │  ┌──────────────┐
         │ Pool objects  │────────────┤  │ DeepBook pkg │
         │ BigVector     │            │  │ Sui framework│
         │ slices        │            │  └──────┬───────┘
         └──────────────┘            │         │
                                      ▼         ▼
                              ┌──────────────────────┐
                              │  sui-sandbox MoveVM   │
                              │  iter_orders() PTB    │
                              │  execution            │
                              └──────────┬───────────┘
                                         │
                                         ▼
                              ┌──────────────────────┐
                              │  SandboxOrderbook     │
                              │  (cached price levels)│
                              └──────────┬───────────┘
                                         │
              ┌──────────────────────────┼──────────────────────────┐
              ▼                          ▼                          ▼
     ┌────────────────┐      ┌────────────────────┐     ┌──────────────────┐
     │ GET /orderbook │      │ POST /swap/quote   │     │ POST /swap       │
     │ depth, stats   │      │ walk price levels  │     │ execute + update │
     └────────────────┘      └────────────────────┘     │ session balances │
                                                         └──────────────────┘
```

## Pools Available

| Pool | Mid Price (cp 240M) | Bids | Asks |
|------|---------------------|------|------|
| SUI/USDC | $1.2929 | 118 | 166 |
| DEEP/USDC | $0.0332 | — | — |
| WAL/USDC | $0.1077 | — | — |

## Quick Start

```bash
# Backend (Rust)
cd backend
cp .env.example .env  # Set SUI_GRPC_ENDPOINT and SUI_GRPC_API_KEY
cargo run              # Builds MoveVM orderbooks at startup, serves on :3001

# Frontend (Next.js) — not yet wired up
cd frontend
npm install && npm run dev
```

### Prerequisites

- Rust 1.75+
- Node.js 20+
- Sui gRPC endpoint (for loading Move packages at startup)
- Pre-cached pool state files in `backend/data/` (included in repo)

### Environment Variables

```bash
# backend/.env
SUI_GRPC_ENDPOINT=https://...
SUI_GRPC_API_KEY=...
```

## API Endpoints

### Session Management

| Method | Endpoint | Description |
|--------|----------|-------------|
| POST | `/api/session` | Create a new trading session |
| GET | `/api/session/:id` | Get session info and balances |
| GET | `/api/session/:id/history` | View swap history |
| POST | `/api/session/:id/reset` | Reset to initial balances |

### Trading

| Method | Endpoint | Description |
|--------|----------|-------------|
| POST | `/api/swap` | Execute swap (requires session_id) |
| POST | `/api/swap/quote` | Get quote without executing |
| GET | `/api/balance/:session_id` | Get token balances |
| POST | `/api/faucet` | Mint tokens into session |

### Orderbook

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | `/api/pools` | List available pools |
| GET | `/api/orderbook?pool=sui_usdc` | Full orderbook snapshot |
| GET | `/api/orderbook/depth?pool=sui_usdc` | Binance-style depth |
| GET | `/api/orderbook/stats?pool=sui_usdc` | Pool statistics |

### Example

```bash
# Create a session
curl -X POST http://localhost:3001/api/session -H "Content-Type: application/json" -d '{}'

# Get a swap quote (10 SUI → USDC)
curl -X POST http://localhost:3001/api/swap/quote \
  -H "Content-Type: application/json" \
  -d '{"from_token": "SUI", "to_token": "USDC", "amount": "10000000000"}'

# Execute the swap
curl -X POST http://localhost:3001/api/swap \
  -H "Content-Type: application/json" \
  -d '{"session_id": "YOUR_SESSION_ID", "from_token": "SUI", "to_token": "USDC", "amount": "10000000000"}'
```

## Initial Session Balances

| Token | Amount | Decimals |
|-------|--------|----------|
| SUI | 100 | 9 |
| USDC | 1,000 | 6 |
| DEEP | 100,000 | 6 |
| WAL | 10 | 9 |

## Project Structure

```
.
├── backend/                 # Rust API server (Axum + sui-sandbox)
│   ├── src/
│   │   ├── main.rs              # Server entry, MoveVM startup
│   │   ├── lib.rs               # Library crate root
│   │   ├── api/                 # HTTP endpoints
│   │   │   ├── mod.rs           # Router + AppState
│   │   │   ├── session.rs       # Session CRUD
│   │   │   ├── balance.rs       # Balance queries + faucet
│   │   │   ├── swap.rs          # Swap quote + execution
│   │   │   └── orderbook.rs     # Orderbook depth + stats
│   │   ├── sandbox/             # Core MoveVM logic
│   │   │   ├── orderbook_builder.rs  # MoveVM iter_orders + orderbook build
│   │   │   ├── snowflake_bcs.rs      # JSON→BCS object conversion
│   │   │   ├── state_loader.rs       # Pool config + JSONL loading
│   │   │   └── swap_executor.rs      # Session balances + swap execution
│   │   └── types/               # Error types
│   ├── data/                    # Pre-cached pool state (checkpoint 240M)
│   ├── examples/                # MoveVM test examples
│   ├── scripts/                 # Snowflake export scripts
│   └── docs/                    # Architecture docs
├── frontend/                # Next.js web application (WIP)
├── deepbookv3/              # DeepBook V3 Move source (dependency)
└── sui-sandbox/             # sui-sandbox fork (dependency)
```

## Development

```bash
# Build
cargo build

# Run tests
cargo test

# Run with debug logging
RUST_LOG=debug cargo run

# Test MoveVM orderbook building for all pools
cargo run --example test_all_pools_240m

# Test swap simulation against MoveVM orderbook
cargo run --example test_swap_simulation
```

## Key Design Decisions

- **MoveVM-based orderbook**: Orders are decoded by executing DeepBook's `iter_orders` via PTB in `sui-sandbox`, not by manually parsing BCS. This guarantees correct price extraction.
- **Static checkpoint**: Pool state comes from Snowflake at checkpoint 240M. Orderbooks are built once at startup and cached. This avoids runtime gRPC calls for pool data.
- **Session isolation**: Each user session has independent balances. Swaps debit/credit the session without mutating the shared orderbook.
- **gRPC for packages only**: The Sui gRPC endpoint is only used at startup to load Move packages (DeepBook, Sui framework). All pool state comes from pre-cached Snowflake data.

## License

MIT
