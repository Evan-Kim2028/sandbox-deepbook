# DeepBook V3 State Loading from Snowflake

## Overview

This document explains how to load DeepBook V3 pool state from Snowflake for local Move VM execution. The goal is to call `deepbook::order_query::iter_orders` through the Move VM using **only Snowflake data**.

## What's Working (as of 2026-02-05)

### Checkpoint ~149.9M (Simple - 8 objects)
- 8 objects loaded from JSONL
- 55 bid levels, 60 ask levels
- Mid price: $3.468550, Spread: 1 bps
- Move VM executes `iter_orders` correctly

### Checkpoint 240M (Full - 24 objects)
- All 24 objects verified to exist in Snowflake
- 10 asks leaf slices + 10 bids leaf slices
- Use `analytics_db_v2.CHAINDATA_MAINNET.OBJECT` (NOT OBJECT_PARQUET2)

## DeepBook V3 Object Hierarchy

```
Pool Wrapper (0xe05dafb5...)
    └── PoolInner (dynamic field, name=1)
            ├── asks: BigVector
            │       ├── root_id: UID (points to inner node)
            │       └── depth: 1 (one level of inner nodes above leaves)
            │
            └── bids: BigVector
                    ├── root_id: UID (points to inner node)
                    └── depth: 1

BigVector Structure (depth=1):
    Inner Node (Slice<u64>)
        └── vals: [slice_name_1, slice_name_2, ...]
                      │
                      └── Each points to a Leaf Node (Slice<Order>)
```

## Key Object IDs (SUI/USDC Pool)

| Object | ID | Notes |
|--------|-----|-------|
| Pool Wrapper | `0xe05dafb5133bcffb8d59f4e12465dc0e9faeaa05e3e342a08fe135800e3e4407` | Entry point |
| Pool Inner UID | `0x50997b5f1f6401674d3d881a61e09a71776ee19cd8b83114a0a21b3a82f130b5` | Parent for PoolInner dynamic field |
| Asks BigVector | `0x5f8f0e3a2728a161e529ecacdfdface88b2fa669279aa699afd5d6b462c68466` | Parent for ask slices |
| Bids BigVector | `0x090a8eae3204c76e36eebf3440cbde577e062953391760c37c363530fc1de246` | Parent for bid slices |

## Critical Lessons Learned

### 1. All Slices Must Be Present

The Move VM's `iter_orders` function traverses the entire BigVector. If ANY slice is missing, execution fails with:
```
ABORTED { code: 1, ... } in big_vector module
```

**Solution**: Choose a checkpoint where all referenced slices exist in Snowflake.

### 2. Snowflake Stores Incremental Changes, Not Snapshots

`OBJECT_PARQUET2` contains object versions at specific checkpoints when they changed. To find the latest version of an object at checkpoint X:
- The object might have been last modified at checkpoint X-1000
- You need to find the MAX(VERSION) where CHECKPOINT <= X

### 3. QUALIFY ROW_NUMBER is Expensive

**Bad (slow):**
```sql
SELECT * FROM OBJECT_PARQUET2
WHERE OBJECT_ID = '0x...'
AND CHECKPOINT <= 240000000
QUALIFY ROW_NUMBER() OVER (PARTITION BY OBJECT_ID ORDER BY VERSION DESC) = 1
```

**Good (fast):**
```sql
-- First get the max version
SELECT MAX(VERSION) as max_version FROM OBJECT_PARQUET2
WHERE OBJECT_ID = '0x...' AND CHECKPOINT <= 240000000;

-- Then fetch that specific version
SELECT * FROM OBJECT_PARQUET2
WHERE OBJECT_ID = '0x...' AND VERSION = <max_version>;
```

### 4. Use TIMESTAMP_MS for Faster Filtering

The OBJECT_PARQUET2 table is partitioned by time. Adding a TIMESTAMP_MS filter dramatically speeds up queries:

```sql
-- Calculate approximate timestamp for checkpoint 240M
-- Checkpoint 240M ≈ late 2024 timeframe
WHERE TIMESTAMP_MS >= 1700000000000  -- Nov 2023
AND TIMESTAMP_MS <= 1735689600000    -- Jan 2025
AND CHECKPOINT <= 240000000
```

### 5. Inner Node Values Determine Required Slices

The inner node's `vals` array contains the slice names (u64) that must be fetched:
```json
{
  "vals": [382, 811687]  // Must fetch slices named 382 and 811687
}
```

## Query Strategy for Efficiency

### Step 1: Get Pool Objects
```sql
-- Get Pool Wrapper and PoolInner at target checkpoint
WITH target_versions AS (
    SELECT OBJECT_ID, MAX(VERSION) as max_version
    FROM OBJECT_PARQUET2
    WHERE OBJECT_ID IN (
        '0xe05dafb5133bcffb8d59f4e12465dc0e9faeaa05e3e342a08fe135800e3e4407',  -- Pool Wrapper
        '0x5c44ceb4c4e8ebb76813c729f8681a449ed1831129ac6e1cf966c7fcefe7dddb'   -- PoolInner
    )
    AND CHECKPOINT <= 240000000
    AND TIMESTAMP_MS BETWEEN 1700000000000 AND 1735689600000
    GROUP BY OBJECT_ID
)
SELECT o.*
FROM OBJECT_PARQUET2 o
JOIN target_versions tv ON o.OBJECT_ID = tv.OBJECT_ID AND o.VERSION = tv.max_version
WHERE o.OBJECT_JSON IS NOT NULL;
```

### Step 2: Get Inner Nodes (to find slice names)
```sql
-- Inner nodes are dynamic fields of the BigVector UIDs
WITH target_versions AS (
    SELECT OBJECT_ID, MAX(VERSION) as max_version
    FROM OBJECT_PARQUET2
    WHERE OWNER_ADDRESS IN (
        '0x5f8f0e3a2728a161e529ecacdfdface88b2fa669279aa699afd5d6b462c68466',  -- Asks BigVector
        '0x090a8eae3204c76e36eebf3440cbde577e062953391760c37c363530fc1de246'   -- Bids BigVector
    )
    AND STRUCT_TAG LIKE '%Slice<u64>%'
    AND CHECKPOINT <= 240000000
    GROUP BY OBJECT_ID
)
SELECT o.*
FROM OBJECT_PARQUET2 o
JOIN target_versions tv ON o.OBJECT_ID = tv.OBJECT_ID AND o.VERSION = tv.max_version;
```

### Step 3: Get Leaf Slices (Order data)
```sql
-- After parsing inner node vals, fetch each leaf slice
-- Use the same MAX(VERSION) pattern for each slice
```

## Gotchas and Edge Cases

### 1. Missing Slices at Recent Checkpoints
At checkpoint 241M, the asks BigVector had 11 slices but 2 were missing from Snowflake (3727881, 3727913). Solution: Use an earlier checkpoint or wait for data backfill.

### 2. Object ID vs Owner Address
- `OBJECT_ID`: The object's unique identifier
- `OWNER_ADDRESS`: For dynamic fields, this is the PARENT object's UID
- To find dynamic field children, query by `OWNER_ADDRESS = <parent_uid>`

### 3. Slice Name Encoding
The dynamic field "name" for slices is a u64 encoded in BCS. The name in the inner node's `vals` array matches this.

### 4. Version vs Checkpoint
- `VERSION`: Object's version number (increments with each mutation)
- `CHECKPOINT`: The Sui checkpoint where this version was recorded
- An object at checkpoint 240M might have VERSION from an earlier checkpoint

### 5. BCS Conversion Requirements
The Rust code converts JSON to BCS for Move VM execution. Key type mappings:
- `Slice<Order>`: Contains order data with tick prices, quantities
- `Slice<u64>`: Inner node containing child slice names
- `PoolInner`: Contains BigVector root IDs and pool configuration

## File Locations

| File | Purpose |
|------|---------|
| `data/sui_usdc_state_cp149.9M.jsonl` | Working state at checkpoint ~149.9M |
| `data/sui_usdc_state_cp240M.jsonl` | State at checkpoint 240M (to be created) |
| `src/sandbox/orderbook_builder.rs` | Main orderbook building logic |
| `src/sandbox/state_loader.rs` | Loads state from JSONL files |
| `src/sandbox/snowflake_bcs.rs` | JSON to BCS conversion |
| `examples/test_orderbook.rs` | Integration test |
| `scripts/fetch_state_efficient.py` | Efficient Snowflake queries (generic) |
| `scripts/export_state_240m.py` | Export script for checkpoint 240M |
| `docs/SNOWFLAKE_OPTIMIZATION_PROPOSAL.md` | Proposed table optimizations |

## Snowflake Table Selection

**CRITICAL**: Use the correct table for your checkpoint range:

| Table | Checkpoint Range | Has OBJECT_JSON |
|-------|------------------|-----------------|
| `PIPELINE_V2_GROOT_DB.PIPELINE_V2_GROOT_SCHEMA.OBJECT_PARQUET2` | Up to ~114M | Yes |
| `analytics_db_v2.CHAINDATA_MAINNET.OBJECT` | Up to 241M+ | Yes |
| `PIPELINE_V2_GROOT_DB.PIPELINE_V2_GROOT_SCHEMA.OBJECT_LATEST` | Latest only | No (summary) |

For checkpoint 240M, use `analytics_db_v2.CHAINDATA_MAINNET.OBJECT`.

## Testing

```bash
# Run the integration test
cargo run --example test_orderbook

# Expected output:
# - 8 objects loaded
# - 55 bid levels, 60 ask levels
# - Mid price around $3.47
# - "Book is valid (best bid < best ask)"
```

## Future Improvements

1. **Automated Slice Discovery**: Query Snowflake to find checkpoints where all slices exist
2. **Incremental Updates**: Track orderbook changes over time
3. **Multiple Pools**: Support for other trading pairs (DEEP/USDC, etc.)
4. **Real-time Sync**: Periodic refresh from latest Snowflake data
