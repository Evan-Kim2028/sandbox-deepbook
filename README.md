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

## Getting Started

### Prerequisites

- **Rust 1.75+** ([install](https://rustup.rs))
- **Sui CLI 1.64+** (`sui --version`)
- **Git with submodule support**

### 1. Clone and configure

```bash
git clone --recurse-submodules https://github.com/Evan-Kim2028/sandbox-deepbook.git
cd sandbox-deepbook

# If you cloned without --recurse-submodules (or pulled new submodule pointers)
git submodule update --init --recursive

cd backend

# Set up environment (uses public Sui gRPC endpoint by default)
cp .env.example .env
```

The default `.env` uses `https://archive.mainnet.sui.io:443` for fetching Move packages at startup. No API key needed.
`deepbookv3/` is a pinned git submodule and is required for router Move contract compilation.

### 2. Build and run the server

```bash
cargo run
```

On first run, Cargo will download and compile dependencies (~2-3 min). The server then:
1. Loads pool state from `data/*.jsonl` files (included in repo, checkpoint 240M)
2. Fetches DeepBook + Sui framework packages via gRPC (~5s)
3. Builds orderbooks by executing `iter_orders` in the Move VM
4. Compiles the local router Move contract (`contracts/router`) with `sui move build --environment mainnet`
5. Starts serving on `http://localhost:3001`

You'll see output like:
```
Pool registry ready: 3/3 pools loaded
Building MoveVM orderbooks from checkpoint 240M state...
  SUI/USDC built: 118 bids, 166 asks, mid=$1.292900
MoveVM orderbooks built: 3 pools ready
Starting server on 0.0.0.0:3001
```

### 3. Try a swap

```bash
# Create a trading session (gives you 100 SUI, 1000 USDC, etc.)
curl -s -X POST http://localhost:3001/api/session \
  -H "Content-Type: application/json" -d '{}' | jq .session_id

# Get a quote: sell 10 SUI for USDC
curl -s -X POST http://localhost:3001/api/swap/quote \
  -H "Content-Type: application/json" \
  -d '{"from_token": "SUI", "to_token": "USDC", "amount": "10000000000"}' | jq

# Execute the swap (paste your session_id from step 1)
curl -s -X POST http://localhost:3001/api/swap \
  -H "Content-Type: application/json" \
  -d '{"session_id": "YOUR_SESSION_ID", "from_token": "SUI", "to_token": "USDC", "amount": "10000000000"}' | jq
```

The `amount` field is in raw token units (10 SUI = `10000000000` since SUI has 9 decimals).

## Two-Hop Quote Behavior

- Two-hop quotes (`TOKEN_A -> USDC -> TOKEN_B`) try the MoveVM router first.
- For very small inputs, DeepBook may abort in `get_quantity_out` (lot-size/rounding edge case).
- When that happens, the API automatically falls back to Rust simulation over the same MoveVM-built orderbook snapshots.
- Swap execution remains in the local sandbox backend flow and uses the same session/orderbook state.

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
│   ├── scripts/                 # Snowflake export scripts (Python)
│   └── docs/                    # Architecture docs
└── frontend/                # Next.js web application (WIP)
```

Dependencies (`sui-sandbox`, `move-core-types`, etc.) are fetched automatically by Cargo from GitHub.

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
