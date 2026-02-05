# DeepBook Sandbox Frontend - Project Plan

## Executive Summary

Build an interactive web application that allows users to experience Sui's DeepBook protocol in a forked mainnet environment. Users can simulate swaps, test PTBs, and understand DeFi mechanics without risking real assets or needing testnet tokens.

---

## Goals & Objectives

### Primary Goals
1. **Demonstrate sui-sandbox capabilities** - Show the power of local Move VM execution with forked mainnet state
2. **Lower barrier to DeepBook experimentation** - No testnet tokens, no gas costs, instant feedback
3. **Create shareable "aha moments"** - A link anyone can visit to experience forked DeFi

### User Experience Goals
Users should leave with clear answers to:
- [x] Mainnet fork? **Yes** - Real DeepBook state, real prices
- [x] No testnet tokens needed? **Yes** - Simulated balances
- [x] Easy to build on DeepBook? **Yes** - Visual PTB builder + outputs

### Success Metrics
- User can complete a USDC → SUI swap within 30 seconds of landing
- PTB output visualization helps users understand what happened
- Zero blockchain knowledge required for basic interactions

---

## Architecture Overview

```
┌─────────────────────────────────────────────────────────────────┐
│                        FRONTEND (Next.js)                       │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────────────┐  │
│  │ Wallet Mock  │  │ Swap UI      │  │ PTB Visualizer       │  │
│  │ (Simulated)  │  │              │  │ (Transaction Output) │  │
│  └──────────────┘  └──────────────┘  └──────────────────────┘  │
└─────────────────────────────────────────────────────────────────┘
                              │
                              │ REST/WebSocket API
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│                     BACKEND (Rust + Axum)                       │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────────────┐  │
│  │ Session Mgmt │  │ API Layer    │  │ State Sync Service   │  │
│  │              │  │              │  │ (Periodic Updates)   │  │
│  └──────────────┘  └──────────────┘  └──────────────────────┘  │
│                              │                                  │
│                              ▼                                  │
│  ┌──────────────────────────────────────────────────────────┐  │
│  │              sui-sandbox (SimulationEnvironment)          │  │
│  │  • Move VM Execution    • Gas Metering                   │  │
│  │  • Object State         • PTB Simulation                 │  │
│  └──────────────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│                     STATE PERSISTENCE                           │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────────────┐  │
│  │ RocksDB/     │  │ Forked State │  │ User Sessions        │  │
│  │ SQLite       │  │ (Mainnet)    │  │ (Redis/Memory)       │  │
│  └──────────────┘  └──────────────┘  └──────────────────────┘  │
└─────────────────────────────────────────────────────────────────┘
                              │
                              │ gRPC (periodic sync)
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│                      SUI MAINNET (Read-Only)                    │
│  • DeepBook V3 Packages    • Pool Objects                       │
│  • Token Metadata          • Current Prices                     │
└─────────────────────────────────────────────────────────────────┘
```

---

## Technical Components

### 1. Backend Service (Rust)

**Core Responsibilities:**
- Wrap sui-sandbox as an HTTP API
- Manage per-session simulation environments
- Handle state forking and persistence
- Execute PTBs and return structured results

**Key Modules:**

```
backend/
├── src/
│   ├── main.rs                 # Axum server entrypoint
│   ├── api/
│   │   ├── mod.rs
│   │   ├── swap.rs             # POST /api/swap
│   │   ├── balance.rs          # GET /api/balance/:address
│   │   ├── session.rs          # POST /api/session (create sandbox)
│   │   └── ptb.rs              # POST /api/ptb/execute
│   ├── sandbox/
│   │   ├── mod.rs
│   │   ├── environment.rs      # SimulationEnvironment wrapper
│   │   ├── deepbook.rs         # DeepBook-specific PTB builders
│   │   └── state_sync.rs       # Periodic mainnet state refresh
│   ├── storage/
│   │   ├── mod.rs
│   │   ├── rocks.rs            # RocksDB persistence
│   │   └── session.rs          # User session management
│   └── types/
│       ├── mod.rs
│       └── responses.rs        # API response types
├── Cargo.toml
└── .env.example
```

**Dependencies:**
```toml
[dependencies]
sui-sandbox = { git = "https://github.com/Evan-Kim2028/sui-sandbox" }
axum = "0.7"
tokio = { version = "1", features = ["full"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
rocksdb = "0.22"
tower-http = { version = "0.5", features = ["cors"] }
```

### 2. Frontend Application (Next.js)

**Core Features:**
- Mock wallet with simulated balances (SUI, USDC, WAL)
- Swap interface (USDC ↔ SUI via DeepBook)
- PTB output visualizer
- Transaction history

**Key Components:**

```
frontend/
├── src/
│   ├── app/
│   │   ├── page.tsx            # Landing + Swap UI
│   │   └── layout.tsx
│   ├── components/
│   │   ├── WalletMock.tsx      # Simulated wallet display
│   │   ├── SwapCard.tsx        # Main swap interface
│   │   ├── PTBVisualizer.tsx   # Transaction breakdown
│   │   ├── BalanceDisplay.tsx  # Token balances
│   │   └── TransactionHistory.tsx
│   ├── hooks/
│   │   ├── useSandbox.ts       # API interactions
│   │   └── useSession.ts       # Session management
│   └── lib/
│       ├── api.ts              # Backend API client
│       └── types.ts            # Shared types
├── package.json
└── next.config.js
```

**Tech Stack:**
- Next.js 14 (App Router)
- TailwindCSS
- shadcn/ui components
- React Query for data fetching

### 3. State Management Strategy

**Challenge:** How to keep forked state fresh while allowing user modifications?

**Solution: Layered State Model**

```
┌─────────────────────────────────────┐
│     User Session State (Ephemeral)  │  ← User's transactions
├─────────────────────────────────────┤
│     Forked Base State (Daily Sync)  │  ← Mainnet snapshot
├─────────────────────────────────────┤
│     DeepBook Packages (Stable)      │  ← Rarely changes
└─────────────────────────────────────┘
```

**State Sync Strategy:**
1. **On Server Start:** Load last snapshot from RocksDB
2. **Daily (or configurable):** Refresh forked state from mainnet
3. **Per User Session:** Create overlay on base state for user transactions
4. **Session Timeout:** Discard user overlay after inactivity

**Objects to Fork:**
- DeepBook V3 packages
- USDC/SUI/WAL pool objects
- Price oracles (if applicable)
- Coin metadata

---

## API Design

### Session Management

```http
POST /api/session
Response: { session_id: string, expires_at: timestamp }

DELETE /api/session/:id
Response: { success: boolean }
```

### Wallet Operations

```http
GET /api/balance/:session_id
Response: {
  sui: "1000000000",   // 1 SUI in MIST
  usdc: "100000000",   // 100 USDC
  wal: "50000000000"   // 50 WAL
}

POST /api/faucet
Body: { session_id: string, token: "sui" | "usdc" | "wal", amount: string }
Response: { new_balance: string, tx_effects: {...} }
```

### Swap Operations

```http
POST /api/swap
Body: {
  session_id: string,
  from_token: "usdc",
  to_token: "sui",
  amount: "10000000",  // 10 USDC
  slippage_bps: 50     // 0.5%
}
Response: {
  success: boolean,
  ptb: {
    commands: [...],
    inputs: [...],
    gas_used: "1234567",
    effects: {...}
  },
  output_amount: "2500000000",  // SUI received
  execution_time_ms: 45
}
```

### PTB Inspection

```http
POST /api/ptb/simulate
Body: {
  session_id: string,
  ptb_json: {...}  // Raw PTB structure
}
Response: {
  success: boolean,
  gas_estimate: string,
  effects_preview: {...},
  error?: string
}
```

---

## Implementation Phases

### Phase 1: Foundation (Week 1-2)
**Goal:** Prove the core loop works

- [ ] Set up Rust backend with Axum
- [ ] Integrate sui-sandbox as dependency
- [ ] Implement basic state forking for DeepBook
- [ ] Create session management (in-memory first)
- [ ] Build minimal API: `/session`, `/balance`, `/swap`
- [ ] Test USDC → SUI swap via CLI/curl

**Deliverable:** Backend that can execute a forked DeepBook swap

### Phase 2: Frontend MVP (Week 3-4)
**Goal:** Visual swap interface

- [ ] Set up Next.js project with TailwindCSS
- [ ] Build WalletMock component (static balances first)
- [ ] Create SwapCard component
- [ ] Wire up API integration
- [ ] Basic PTB output display (JSON view)
- [ ] Deploy frontend to Vercel

**Deliverable:** Working swap UI connected to backend

### Phase 3: State Persistence (Week 5-6)
**Goal:** Reliable forked state management

- [ ] Integrate RocksDB for state persistence
- [ ] Implement state sync service (periodic refresh)
- [ ] Add session persistence (survive restarts)
- [ ] Handle state versioning
- [ ] Add health checks and monitoring

**Deliverable:** Backend survives restarts, state stays fresh

### Phase 4: Polish & UX (Week 7-8)
**Goal:** Production-ready experience

- [ ] PTB Visualizer (graphical breakdown)
- [ ] Transaction history
- [ ] Faucet functionality (get test tokens)
- [ ] Error handling and loading states
- [ ] Mobile responsiveness
- [ ] Performance optimization

**Deliverable:** Shareable demo ready for distribution

### Phase 5: Extended Features (Future)
- Multi-pool support (add WAL pairs)
- Custom PTB builder UI
- Export PTB for mainnet execution
- Developer API documentation
- Rate limiting and abuse prevention

---

## Open Questions & Risks

### Technical Risks

| Risk | Mitigation |
|------|------------|
| State drift (forked state becomes stale) | Daily sync + version tracking |
| gRPC API rate limits | Caching layer, request batching |
| Session memory bloat | TTL expiration, LRU eviction |
| Sui protocol upgrades | Pin to known-working versions |

### Open Questions

1. **State sync frequency:** Daily? Hourly? On-demand?
   - *Proposal:* Daily baseline + option for manual refresh

2. **User isolation:** Full isolation or shared base state?
   - *Proposal:* Shared base + isolated overlays

3. **Deployment topology:** Monolith or separate services?
   - *Proposal:* Start monolith, split if needed

4. **Authentication:** Anonymous sessions or require login?
   - *Proposal:* Anonymous with rate limiting, optional login for persistence

---

## Required Infrastructure

### Development
- Rust toolchain (1.75+)
- Node.js 20+
- Docker (for local RocksDB)
- Sui gRPC endpoint access

### Production
- VPS or container hosting (Fly.io, Railway, or self-hosted)
- RocksDB persistent volume
- Environment variables for gRPC credentials
- CDN for frontend (Vercel/Cloudflare)

---

## State Hydration Strategy (Validated)

### Key Finding: Snowflake + sui-sandbox Hybrid

Based on Snowflake exploration (see `docs/SNOWFLAKE_FINDINGS.md`):

1. **Snowflake has fresh object data** at `analytics_db_v2.chaindata_mainnet.object`
2. **DeepBook V3 pools use wrapper architecture** - Pool object → inner dynamic fields
3. **sui-sandbox can fetch objects via gRPC** - handles dynamic field resolution automatically

### Recommended Approach: Test sui-sandbox Native First

```rust
// sui-sandbox already handles forking
let env = SimulationEnvironment::new_from_mainnet_fork(&[
    "0xe05dafb5133bcffb8d59f4e12465dc0e9faeaa05e3e342a08fe135800e3e4407", // SUI/USDC pool
]).await?;
```

**If cold-start is too slow**, implement Snowflake pre-warming:
1. Query pool IDs from `DEEPBOOK_POOLS_CREATED`
2. Pre-fetch object metadata from Snowflake
3. Cache in RocksDB for faster startup

### DeepBook V3 Package
```
V3 Upgrade Package: 0xcaf6ba059d539a97646d47f0b9ddf843e138d215e2a12ca1f4585d386f7aec3a
Checkpoint: 134,587,749 (April 2025)

Pool Type Pattern: 0x2c8d603bc51326b8c13cef9dd07031a408a48dddb541963357661df5d3204809::pool::Pool<...>
```

---

## DeepBook Integration Details

### Validated Pool Objects
```
SUI/USDC Pool: 0xe05dafb5133bcffb8d59f4e12465dc0e9faeaa05e3e342a08fe135800e3e4407
  - Type: Pool<SUI, USDC>
  - Owner: Shared
  - Initial Shared Version: 389750322
```

### Packages to Fork
```
DeepBook Core: 0x2c8d603bc51326b8c13cef9dd07031a408a48dddb541963357661df5d3204809
DeepBook V3:   0xcaf6ba059d539a97646d47f0b9ddf843e138d215e2a12ca1f4585d386f7aec3a
USDC:          0xdba34672e30cb065b1f93e3ab55318768fd6fef66c15942c9f7cb846e2f900e7
SUI:           0x2::sui::SUI
WAL:           0x356a26eb9e012a68958082340d4c4116e7f55615cf27affcff209cf0ae544f59
```

### Key Objects
- Pool: `SUI_USDC_POOL` object ID
- State: DeepBook global state object
- Price feeds: Any oracle dependencies

### PTB Structure for Swap
```
1. SplitCoins(coin_input, [amount])
2. MoveCall(deepbook::swap_exact_base_for_quote, [...])
3. TransferObjects([output_coin], sender)
```

---

## Next Steps

### ✅ Completed
1. **Snowflake Object Discovery** - Identified all ~852 objects needed for SUI/USDC pool
2. **Backend Scaffolding** - Axum server with StateLoader and orderbook endpoints
3. **Frontend UI** - SwapCard, OrderBook, FaucetModal, ActivityFeed components
4. **sui-sandbox Fork** - Branch `feat/sandbox-frontend-api` created from PR #19

### 🔄 In Progress
1. **Export state from Snowflake** - Run `backend/sql/export_deepbook_state.sql`
2. **Wire up JsonToBcsConverter** - Convert Snowflake JSON to BCS bytes
3. **Integrate SimulationEnvironment** - Load objects and execute PTBs

### 📋 TODO
1. **Test state loading** - Load exported JSON into backend, verify orderbook parsing
2. **Add BCS conversion** - Use sui-sandbox JsonToBcsConverter
3. **Execute PTBs** - Wire up swap execution with SimulationEnvironment
4. **Connect frontend** - Update frontend to use real orderbook data

---

## sui-sandbox Integration

### Branch Setup

We've forked from PR #19 which contains the JSON→BCS reconstruction logic:

```bash
# Local clone at: /Users/evandekim/Documents/sui-sandbox-frontend
# Branch: feat/sandbox-frontend-api (forked from feat/deepbook-margin-state-example)
```

### Key Files from PR #19

| File | Purpose |
|------|---------|
| `examples/deepbook_margin_state/json_to_bcs.rs` | **JSON→BCS converter** using bytecode layouts |
| `examples/deepbook_margin_state/common.rs` | Common utilities for state loading |
| `examples/deepbook_margin_state/main.rs` | Example of historical state reconstruction |
| `crates/sui-sandbox-core/src/utilities/generic_patcher.rs` | BcsEncoder, LayoutRegistry |

### JsonToBcsConverter API

```rust
use sui_sandbox_core::utilities::generic_patcher::*;

let mut converter = JsonToBcsConverter::new();
converter.add_modules_from_bytes(&bytecode_list)?;

// Convert Snowflake OBJECT_JSON to BCS
let bcs_bytes = converter.convert(
    "0x2c8d...::pool::Pool<0x2::sui::SUI, 0xdba3...::usdc::USDC>",
    &object_json
)?;
```

### Integration Path

1. **Snowflake** → Query OBJECT_JSON for DeepBook pools
2. **JsonToBcsConverter** → Convert to BCS bytes
3. **SimulationEnvironment** → Load objects into local VM
4. **Axum API** → Expose swap/balance endpoints

---

## References

- [sui-sandbox GitHub](https://github.com/Evan-Kim2028/sui-sandbox)
- [PR #19: DeepBook margin state example](https://github.com/Evan-Kim2028/sui-sandbox/pull/19)
- [DeepBook V3 Documentation](https://deepbook.sui.io/)
- [Sui Move Reference](https://docs.sui.io/concepts/sui-move-concepts)
- [Axum Web Framework](https://github.com/tokio-rs/axum)
