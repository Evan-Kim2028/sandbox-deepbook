# Snowflake Data Discovery Findings

## Key Finding: Snowflake CAN Provide Full Object State

### Working Data Source
- **Table**: `analytics_db_v2.chaindata_mainnet.object`
- **Freshness**: Near real-time (checkpoint 241M+ as of Feb 2026)
- **Contains**: Full object data including `OBJECT_JSON`, `BCS_LENGTH`, `DIGEST`, `TYPE`

### DeepBook V3 Information

**V3 Package (upgrade package)**:
```
0xcaf6ba059d539a97646d47f0b9ddf843e138d215e2a12ca1f4585d386f7aec3a
Checkpoint: 134,587,749 (April 2025)
```

**Pool Type Pattern** (in TYPE column):
```
0x2c8d603bc51326b8c13cef9dd07031a408a48dddb541963357661df5d3204809::pool::Pool<...>
```
Note: V3 pools still use the original DeepBook package type in their object TYPE.

### Sample Pool Object (SUI/USDC)

```
OBJECT_ID: 0xe05dafb5133bcffb8d59f4e12465dc0e9faeaa05e3e342a08fe135800e3e4407
VERSION: 775620872
TYPE: 0x2c8d603bc51326b8c13cef9dd07031a408a48dddb541963357661df5d3204809::pool::Pool<0x2::sui::SUI, 0xdba34672e30cb065b1f93e3ab55318768fd6fef66c15942c9f7cb846e2f900e7::usdc::USDC>
OWNER_TYPE: Shared
INITIAL_SHARED_VERSION: 389750322
BCS_LENGTH: 263

OBJECT_JSON:
{
  "id": {"id": "0xe05dafb5133bcffb8d59f4e12465dc0e9faeaa05e3e342a08fe135800e3e4407"},
  "inner": {
    "id": {"id": "0x50997b5f1f6401674d3d881a61e09a71776ee19cd8b83114a0a21b3a82f130b5"},
    "version": "1"
  }
}
```

### Critical Architecture Insight

DeepBook pools are **versioned wrappers**:
1. The `Pool` object is a thin wrapper with an `inner` reference
2. The actual order book data (bids, asks, balances) lives in dynamic fields
3. Dynamic fields are owned by the parent object, not stored as separate rows

**This means**: To fully hydrate a DeepBook pool, we need:
- The Pool wrapper object
- All dynamic field children (resolved via parent ID)
- Package dependencies

---

## Query Patterns That Work

### Find DeepBook Pools (Optimized)
```sql
SELECT OBJECT_ID, VERSION, TYPE, CHECKPOINT, BCS_LENGTH, OBJECT_JSON
FROM analytics_db_v2.chaindata_mainnet.object
WHERE TYPE LIKE '%pool::Pool<%'
  AND CHECKPOINT > 134000000  -- V3 era
  AND OBJECT_STATUS = 'Mutated'
ORDER BY VERSION DESC
LIMIT 50
```

### Get Pool Events (for IDs)
```sql
SELECT POOL_ID, BASE_ASSET, QUOTE_ASSET, PACKAGE
FROM PIPELINE_V2_GROOT_DB.PIPELINE_V2_GROOT_SCHEMA.DEEPBOOK_POOLS_CREATED
WHERE PACKAGE = '0xcaf6ba059d539a97646d47f0b9ddf843e138d215e2a12ca1f4585d386f7aec3a'  -- V3 only
ORDER BY TIMESTAMP_MS DESC
```

### Get Object by ID (use CHECKPOINT filter)
```sql
SELECT *
FROM analytics_db_v2.chaindata_mainnet.object
WHERE OBJECT_ID = '<id>'
  AND CHECKPOINT > <recent_checkpoint>
ORDER BY VERSION DESC
LIMIT 1
```

---

## V3 Pools Identified

| Pool ID | Base | Quote | Purpose |
|---------|------|-------|---------|
| 0xc5d273... | FDUSD | USDC | Stablecoin |
| 0xd90e0a... | UP | SUI | Token/SUI |
| 0xd94745... | LBTC | USDC | BTC/stable |
| 0x20b9a3... | XBTC | USDC | BTC/stable |
| 0xa01557... | TLP | SUI | Token/SUI |

---

## Data Not Available in Snowflake

1. **Raw BCS bytes** - Only `BCS_LENGTH`, not actual bytes
2. **Dynamic field children** - Need separate query or RPC
3. **Package bytecode** - Not in object table

---

## Recommended Hydration Strategy

### Option A: Snowflake + RPC Hybrid (Fastest)

1. **Discovery via Snowflake**:
   - Query `DEEPBOOK_POOLS_CREATED` for V3 pool IDs
   - Query `analytics_db_v2.chaindata_mainnet.object` for object metadata
   - Get VERSION and DIGEST for each pool

2. **Fetch via RPC**:
   - Use sui-sandbox's built-in gRPC fetcher
   - Pass object IDs + versions discovered from Snowflake
   - Let sandbox handle dynamic field resolution

### Option B: Pure Snowflake Export

Export objects to JSON, then convert to BCS using a Rust tool:

```sql
-- Export all DeepBook objects
SELECT
    OBJECT_ID,
    VERSION,
    DIGEST,
    TYPE,
    OBJECT_JSON,
    INITIAL_SHARED_VERSION
FROM analytics_db_v2.chaindata_mainnet.object
WHERE TYPE LIKE '%2c8d603bc51326b8c13cef9dd07031a408a48dddb541963357661df5d3204809%Pool%'
  AND OBJECT_STATUS != 'Deleted'
  AND CHECKPOINT > 134000000
```

Then in Rust:
1. Parse OBJECT_JSON
2. Reconstruct Move struct
3. Serialize to BCS
4. Load into SimulationEnvironment

### Option C: sui-sandbox Native (Simplest)

Just use sui-sandbox's existing fork capability:
```rust
// sui-sandbox already handles this
let env = SimulationEnvironment::new_from_mainnet_fork(&[
    "0xe05dafb5133bcffb8d59f4e12465dc0e9faeaa05e3e342a08fe135800e3e4407", // SUI/USDC pool
]).await?;
```

This fetches objects + dependencies automatically via gRPC.

---

## Next Steps

1. [ ] Test sui-sandbox native forking with a DeepBook pool ID
2. [ ] Benchmark cold-start latency
3. [ ] If too slow, implement Snowflake-based pre-warming
4. [ ] Cache fetched objects for faster subsequent loads
