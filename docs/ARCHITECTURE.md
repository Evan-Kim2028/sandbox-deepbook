# DeepBook Sandbox Architecture

## Overview

This document describes how we reconstruct historical DeepBook orderbook state using the sui-sandbox to execute Move view functions against forked mainnet state.

## Why sui-sandbox?

DeepBook V3 uses complex data structures (BigVector for orderbooks) with encoded order IDs. Manual BCS parsing is error-prone. By using sui-sandbox, we:

1. **Execute actual Move code** - The DeepBook contract interprets its own data structures correctly
2. **Get accurate prices** - Order IDs are decoded by `utils::decode_order_id()` in the contract
3. **Avoid parsing errors** - No manual bit manipulation or BCS decoding

## Architecture Flow

```
┌─────────────────────────────────────────────────────────────────────┐
│                        Historical State Source                       │
│                                                                      │
│  Snowflake Query (checkpoint-specific)                              │
│    └── ANALYTICS_DB_V2.CHAINDATA_MAINNET.OBJECT                     │
│        └── Filter: CHECKPOINT <= target_checkpoint                  │
│        └── Latest version per OBJECT_ID                             │
└─────────────────────────────────────────────────────────────────────┘
                                    │
                                    ▼
┌─────────────────────────────────────────────────────────────────────┐
│                         State Export (JSONL)                         │
│                                                                      │
│  Objects exported:                                                   │
│    1. Pool Wrapper (e.g., 0xe05d... for SUI/USDC)                   │
│    2. Pool Inner UID (e.g., 0x5099...)                              │
│    3. Bids BigVector (e.g., 0x090a...)                              │
│    4. Asks BigVector (e.g., 0x5f8f...)                              │
│    5. All BigVector slices (dynamic fields owned by bids/asks)      │
│                                                                      │
│  Each line: { object_id, version, owner_address, bcs_data, type }   │
└─────────────────────────────────────────────────────────────────────┘
                                    │
                                    ▼
┌─────────────────────────────────────────────────────────────────────┐
│                      sui-sandbox SimulationEnvironment               │
│                                                                      │
│  1. Load Packages:                                                   │
│     - 0x1 (Move Stdlib)                                             │
│     - 0x2 (Sui Framework)                                           │
│     - 0x2c8d... (DeepBook V3)                                       │
│                                                                      │
│  2. Load Objects:                                                    │
│     - Pool wrapper + inner                                          │
│     - BigVector containers (bids/asks)                              │
│     - All BigVector slices (order data)                             │
│                                                                      │
│  3. Configure child fetcher for dynamic field access                │
└─────────────────────────────────────────────────────────────────────┘
                                    │
                                    ▼
┌─────────────────────────────────────────────────────────────────────┐
│                    Move Call: iter_orders                            │
│                                                                      │
│  Function: deepbook::order_query::iter_orders<BaseAsset, QuoteAsset> │
│                                                                      │
│  Arguments:                                                          │
│    - pool: &Pool<BaseAsset, QuoteAsset>                             │
│    - start_order_id: Option<u128>  (None = start from best)         │
│    - end_order_id: Option<u128>    (None = iterate all)             │
│    - min_expire_timestamp: Option<u64> (None = no filter)           │
│    - limit: u64 (max orders to return)                              │
│    - bids: bool (true = bids, false = asks)                         │
│                                                                      │
│  Returns: OrderPage { orders: vector<Order>, has_next_page: bool }  │
└─────────────────────────────────────────────────────────────────────┘
                                    │
                                    ▼
┌─────────────────────────────────────────────────────────────────────┐
│                         Order Struct                                 │
│                                                                      │
│  struct Order {                                                      │
│    balance_manager_id: ID,                                          │
│    order_id: u128,        // Encodes: side | price | sequence       │
│    client_order_id: u64,                                            │
│    quantity: u64,         // Base units (e.g., MIST for SUI)        │
│    filled_quantity: u64,                                            │
│    fee_is_deep: bool,                                               │
│    order_deep_price: OrderDeepPrice,                                │
│    epoch: u64,                                                      │
│    status: u8,                                                      │
│    expire_timestamp: u64,                                           │
│  }                                                                   │
│                                                                      │
│  Price extraction: utils::decode_order_id(order_id) -> (is_bid, price, seq)
│  Price in USD: price / 1_000_000 (6 decimal places for USDC quote)  │
└─────────────────────────────────────────────────────────────────────┘
                                    │
                                    ▼
┌─────────────────────────────────────────────────────────────────────┐
│                      Orderbook Construction                          │
│                                                                      │
│  1. Call iter_orders(pool, None, None, None, 1000, true)  // bids   │
│  2. Call iter_orders(pool, None, None, None, 1000, false) // asks   │
│  3. For each Order:                                                  │
│     - price = order.price() (decoded from order_id)                 │
│     - remaining_qty = quantity - filled_quantity                    │
│     - Aggregate by price level                                      │
│  4. Sort: bids descending, asks ascending                           │
└─────────────────────────────────────────────────────────────────────┘

## Key Object IDs (SUI/USDC Pool)

| Object | ID |
|--------|-----|
| Pool Wrapper | `0xe05dafb5133bcffb8d59f4e12465dc0e9faeaa05e3e342a08fe135800e3e4407` |
| Pool Inner | `0x50997b5f1f6401674d3d881a61e09a71776ee19cd8b83114a0a21b3a82f130b5` |
| Asks BigVector | `0x5f8f0e3a2728a161e529ecacdfdface88b2fa669279aa699afd5d6b462c68466` |
| Bids BigVector | `0x090a8eae3204c76e36eebf3440cbde577e062953391760c37c363530fc1de246` |
| DeepBook Package | `0x2c8d603bc51326b8c13cef9dd07031a408a48dddb541963357661df5d3204809` |

## Order ID Encoding (DeepBook V3)

```
Bit 127:     Side (0 = bid, 1 = ask)
Bits 64-126: Price (tick value)
Bits 0-63:   Order sequence ID

For bids:  order_id ranges from 0 to (1 << 127)
For asks:  order_id ranges from (1 << 127) to MAX_U128
```

## Price Calculation

```
price_in_quote_units = tick_from_order_id
price_in_usd = tick_from_order_id / 10^quote_decimals

For SUI/USDC (quote_decimals = 6):
  tick = 1_150_000 means price = $1.15
```

## Type Arguments for Pools

| Pool | BaseAsset | QuoteAsset |
|------|-----------|------------|
| SUI/USDC | `0x2::sui::SUI` | `0xdba34...::usdc::USDC` |
| WAL/USDC | `0x356a...::wal::WAL` | `0xdba34...::usdc::USDC` |
| DEEP/USDC | `0xdeeb7...::deep::DEEP` | `0xdba34...::usdc::USDC` |

## Directory Structure

```
backend/
├── src/
│   ├── main.rs              # Entry point, loads state, starts server
│   ├── api/
│   │   ├── mod.rs           # Router configuration
│   │   └── orderbook.rs     # /api/orderbook endpoints
│   └── sandbox/
│       ├── mod.rs           # Module exports
│       ├── state_loader.rs  # JSONL state loading
│       └── orderbook_builder.rs  # sui-sandbox integration (NEW)
├── data/
│   ├── sui_usdc_state_feb2_2026.jsonl
│   ├── wal_usdc_state_feb2_2026.jsonl
│   └── deep_usdc_state_feb2_2026.jsonl
└── Cargo.toml

sui-sandbox/                  # Cloned from github.com/Evan-Kim2028/sui-sandbox
├── crates/
│   └── sui-sandbox-core/    # Core simulation environment
└── examples/
    └── fork_state.rs        # Reference for loading state
```

## API Endpoints

| Endpoint | Description |
|----------|-------------|
| `GET /api/pools` | List all pools and their load status |
| `GET /api/orderbook?pool=sui_usdc` | Raw orderbook with price levels |
| `GET /api/orderbook/depth?pool=sui_usdc` | Binance-style orderbook |
| `GET /api/orderbook/stats?pool=sui_usdc` | Pool statistics |

## Future: Swap Simulation

Once orderbook is correctly built, swap simulation follows:

1. User submits swap request (e.g., sell 100 SUI for USDC)
2. Backend calls `deepbook::pool::place_market_order` in sandbox
3. Sandbox executes against the historical orderbook state
4. Returns simulated fill price, slippage, fees
5. No actual on-chain transaction - pure local simulation
