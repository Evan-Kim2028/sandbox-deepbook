# Snowflake Optimization Proposal for DeepBook State Queries

## Current Challenges

### 1. OBJECT_PARQUET2 is a General-Purpose Table
- Contains ALL Sui objects (billions of rows)
- Not optimized for DeepBook-specific queries
- Scanning for specific objects requires filtering through massive datasets

### 2. Point-in-Time Queries are Expensive
- To get "latest state at checkpoint X", we must:
  - Scan all versions where `CHECKPOINT <= X`
  - Aggregate to find `MAX(VERSION)` per object
  - Join back to get the actual data
- This is O(n) where n = all object versions in range

### 3. No Pre-Computed Snapshots
- Every query recomputes the same work
- No caching of common checkpoint states
- Repeated queries for the same checkpoint are equally expensive

### 4. BigVector Structure Requires Multiple Queries
- Pool → PoolInner → Inner Nodes → Leaf Slices
- 4+ query round-trips minimum
- Must parse JSON to discover required slices

---

## Proposed Solutions

### Option A: DeepBook Object Materialized View

**Concept**: Pre-filtered view containing only DeepBook-related objects.

```sql
CREATE OR REPLACE MATERIALIZED VIEW DEEPBOOK_OBJECTS_MV AS
SELECT
    OBJECT_ID,
    VERSION,
    CHECKPOINT,
    TIMESTAMP_MS,
    STRUCT_TAG,
    OWNER_ADDRESS,
    OBJECT_JSON
FROM OBJECT_PARQUET2
WHERE (
    -- Pool objects
    STRUCT_TAG LIKE '%deepbook%pool%'
    OR STRUCT_TAG LIKE '%deepbook%Pool%'
    -- BigVector slices
    OR STRUCT_TAG LIKE '%big_vector::Slice%'
)
AND OWNER_ADDRESS IN (
    -- Known DeepBook BigVector parents (can expand this list)
    '0x5f8f0e3a2728a161e529ecacdfdface88b2fa669279aa699afd5d6b462c68466',  -- SUI/USDC asks
    '0x090a8eae3204c76e36eebf3440cbde577e062953391760c37c363530fc1de246',  -- SUI/USDC bids
    '0x50997b5f1f6401674d3d881a61e09a71776ee19cd8b83114a0a21b3a82f130b5',  -- SUI/USDC pool inner
    -- Add more pools as needed
);
```

**Pros**:
- Reduces scan size by 99%+
- Auto-refreshes with source data
- No schema changes needed

**Cons**:
- Still requires MAX(VERSION) aggregation
- MV refresh has cost
- Need to know pool IDs upfront

---

### Option B: Checkpoint Snapshot Table

**Concept**: Pre-compute complete DeepBook state at regular intervals.

```sql
CREATE TABLE DEEPBOOK_SNAPSHOTS (
    SNAPSHOT_CHECKPOINT BIGINT,           -- e.g., 240000000, 241000000, etc.
    POOL_ID VARCHAR,                       -- Pool wrapper object ID
    OBJECT_ID VARCHAR,
    OBJECT_TYPE VARCHAR,                   -- 'pool_wrapper', 'pool_inner', 'asks_inner', 'asks_leaf', etc.
    SLICE_NAME BIGINT,                     -- For slices, the dynamic field name
    VERSION BIGINT,
    OBJECT_JSON VARIANT,
    CONSTRAINT pk_snapshot PRIMARY KEY (SNAPSHOT_CHECKPOINT, POOL_ID, OBJECT_ID)
)
CLUSTER BY (SNAPSHOT_CHECKPOINT, POOL_ID);

-- Populate for checkpoint 240M
INSERT INTO DEEPBOOK_SNAPSHOTS
WITH latest_versions AS (
    SELECT OBJECT_ID, MAX(VERSION) as max_version
    FROM OBJECT_PARQUET2
    WHERE CHECKPOINT <= 240000000
    AND OWNER_ADDRESS IN (...known DeepBook parents...)
    GROUP BY OBJECT_ID
)
SELECT
    240000000 as SNAPSHOT_CHECKPOINT,
    '0xe05dafb5...' as POOL_ID,
    o.OBJECT_ID,
    CASE
        WHEN o.STRUCT_TAG LIKE '%Pool<%' THEN 'pool_wrapper'
        WHEN o.STRUCT_TAG LIKE '%PoolInner%' THEN 'pool_inner'
        WHEN o.STRUCT_TAG LIKE '%Slice<u64>%' THEN
            CASE WHEN o.OWNER_ADDRESS = '0x5f8f...' THEN 'asks_inner' ELSE 'bids_inner' END
        WHEN o.STRUCT_TAG LIKE '%Slice<%Order>%' THEN
            CASE WHEN o.OWNER_ADDRESS = '0x5f8f...' THEN 'asks_leaf' ELSE 'bids_leaf' END
    END as OBJECT_TYPE,
    o.OBJECT_JSON:fields:name::BIGINT as SLICE_NAME,
    o.VERSION,
    o.OBJECT_JSON
FROM OBJECT_PARQUET2 o
JOIN latest_versions lv ON o.OBJECT_ID = lv.OBJECT_ID AND o.VERSION = lv.max_version;
```

**Query becomes trivial**:
```sql
SELECT * FROM DEEPBOOK_SNAPSHOTS
WHERE SNAPSHOT_CHECKPOINT = 240000000
AND POOL_ID = '0xe05dafb5...';
```

**Pros**:
- Instant queries (single table scan with clustering)
- Pre-computed, no aggregation needed
- Easy to add new checkpoints

**Cons**:
- Storage cost for each snapshot
- Need to decide snapshot frequency
- Manual refresh process

---

### Option C: Latest State Table (Incrementally Updated)

**Concept**: Maintain current latest version of each DeepBook object.

```sql
CREATE TABLE DEEPBOOK_LATEST_STATE (
    OBJECT_ID VARCHAR PRIMARY KEY,
    POOL_ID VARCHAR,                       -- Which pool this belongs to
    OBJECT_TYPE VARCHAR,
    SLICE_NAME BIGINT,
    VERSION BIGINT,
    CHECKPOINT BIGINT,
    TIMESTAMP_MS BIGINT,
    OBJECT_JSON VARIANT
);

-- Incremental update task (runs on schedule)
MERGE INTO DEEPBOOK_LATEST_STATE target
USING (
    SELECT * FROM OBJECT_PARQUET2
    WHERE CHECKPOINT > (SELECT MAX(CHECKPOINT) FROM DEEPBOOK_LATEST_STATE)
    AND (STRUCT_TAG LIKE '%deepbook%' OR OWNER_ADDRESS IN (...))
) source
ON target.OBJECT_ID = source.OBJECT_ID
WHEN MATCHED AND source.VERSION > target.VERSION THEN
    UPDATE SET VERSION = source.VERSION, CHECKPOINT = source.CHECKPOINT, ...
WHEN NOT MATCHED THEN
    INSERT (...) VALUES (...);
```

**Pros**:
- Always has latest state
- Incremental updates are cheap
- Single query for current orderbook

**Cons**:
- Only gives LATEST state, not historical
- Requires scheduled task maintenance
- May miss intermediate states

---

### Option D: Denormalized Orderbook Table (Recommended)

**Concept**: Flatten the BigVector structure entirely. Store orders directly.

```sql
CREATE TABLE DEEPBOOK_ORDERBOOK_FLAT (
    CHECKPOINT BIGINT,
    POOL_ID VARCHAR,
    SIDE VARCHAR,                          -- 'bid' or 'ask'
    TICK_PRICE BIGINT,                     -- Price level (tick)
    ORDER_ID BIGINT,
    OWNER VARCHAR,
    QUANTITY BIGINT,
    FILLED_QUANTITY BIGINT,
    EXPIRE_TIMESTAMP BIGINT,
    SELF_MATCHING_OPTION NUMBER,
    -- Metadata
    SLICE_OBJECT_ID VARCHAR,
    SLICE_NAME BIGINT,
    EXTRACTED_AT TIMESTAMP_NTZ DEFAULT CURRENT_TIMESTAMP(),
    CONSTRAINT pk_order PRIMARY KEY (CHECKPOINT, POOL_ID, SIDE, ORDER_ID)
)
CLUSTER BY (CHECKPOINT, POOL_ID, SIDE, TICK_PRICE);

-- ETL to populate (parse OBJECT_JSON and flatten orders)
INSERT INTO DEEPBOOK_ORDERBOOK_FLAT
SELECT
    240000000 as CHECKPOINT,
    '0xe05dafb5...' as POOL_ID,
    CASE WHEN OWNER_ADDRESS = '0x5f8f...' THEN 'ask' ELSE 'bid' END as SIDE,
    order_elem.value:tick::BIGINT as TICK_PRICE,
    order_elem.value:order_id::BIGINT as ORDER_ID,
    order_elem.value:owner::VARCHAR as OWNER,
    order_elem.value:quantity::BIGINT as QUANTITY,
    order_elem.value:filled_quantity::BIGINT as FILLED_QUANTITY,
    order_elem.value:expire_timestamp::BIGINT as EXPIRE_TIMESTAMP,
    order_elem.value:self_matching_option::NUMBER as SELF_MATCHING_OPTION,
    OBJECT_ID as SLICE_OBJECT_ID,
    OBJECT_JSON:fields:name::BIGINT as SLICE_NAME
FROM DEEPBOOK_SNAPSHOTS,
LATERAL FLATTEN(input => OBJECT_JSON:fields:value:fields:vals) order_elem
WHERE OBJECT_TYPE IN ('asks_leaf', 'bids_leaf');
```

**Query for full orderbook becomes trivial**:
```sql
SELECT SIDE, TICK_PRICE, SUM(QUANTITY - FILLED_QUANTITY) as TOTAL_QTY, COUNT(*) as ORDER_COUNT
FROM DEEPBOOK_ORDERBOOK_FLAT
WHERE CHECKPOINT = 240000000
AND POOL_ID = '0xe05dafb5...'
AND QUANTITY > FILLED_QUANTITY  -- Only open orders
GROUP BY SIDE, TICK_PRICE
ORDER BY SIDE, TICK_PRICE;
```

**Pros**:
- **Fastest possible queries** - no JSON parsing at query time
- Standard SQL aggregations work directly
- Can add indexes on any column
- Easy to build analytics on top

**Cons**:
- Largest storage footprint
- Requires ETL pipeline to populate
- Need to handle order updates/cancellations

---

## Recommended Approach: Hybrid Solution

### Phase 1: Snapshot Table (Quick Win)
1. Create `DEEPBOOK_SNAPSHOTS` table
2. Populate for key checkpoints (every 1M or 10M)
3. Immediate query speedup with minimal effort

### Phase 2: Denormalized Table (Full Solution)
1. Create `DEEPBOOK_ORDERBOOK_FLAT` table
2. Build ETL to parse and flatten orders
3. Schedule incremental updates

### Phase 3: Real-Time (Future)
1. Stream from Sui indexer directly
2. Update orderbook table in near-real-time
3. Enable live orderbook queries

---

## Storage & Cost Estimates

| Solution | Storage per Pool | Query Time | Maintenance |
|----------|------------------|------------|-------------|
| Current (OBJECT_PARQUET2) | 0 (shared) | 30-120s | None |
| Materialized View | ~10GB | 5-15s | Auto |
| Snapshot Table (per checkpoint) | ~50MB | <1s | Manual |
| Denormalized Flat Table | ~200MB/checkpoint | <100ms | Scheduled |

---

## Implementation Priority

1. **Immediate**: Create snapshot for checkpoint 240M manually
2. **Week 1**: Set up scheduled snapshot generation (every 10M checkpoints)
3. **Week 2**: Build denormalized orderbook ETL
4. **Month 1**: Add more pools, automate everything

---

## Sample Queries After Implementation

### Get full orderbook at checkpoint:
```sql
SELECT * FROM DEEPBOOK_SNAPSHOTS
WHERE SNAPSHOT_CHECKPOINT = 240000000
AND POOL_ID = '0xe05dafb5133bcffb8d59f4e12465dc0e9faeaa05e3e342a08fe135800e3e4407';
-- Returns ~20 rows in <1 second
```

### Get aggregated price levels:
```sql
SELECT SIDE, TICK_PRICE, SUM(QUANTITY - FILLED_QUANTITY) as OPEN_QTY
FROM DEEPBOOK_ORDERBOOK_FLAT
WHERE CHECKPOINT = 240000000 AND POOL_ID = '0xe05dafb5...'
GROUP BY SIDE, TICK_PRICE
ORDER BY SIDE, TICK_PRICE DESC;
-- Returns price levels in <100ms
```

### Compare orderbooks across checkpoints:
```sql
SELECT a.TICK_PRICE, a.OPEN_QTY as QTY_240M, b.OPEN_QTY as QTY_241M
FROM orderbook_240m a
FULL OUTER JOIN orderbook_241m b ON a.TICK_PRICE = b.TICK_PRICE
WHERE a.SIDE = 'bid';
-- Easy historical analysis
```
