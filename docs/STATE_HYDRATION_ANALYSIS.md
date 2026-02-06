# DeepBook State Hydration: Approach Analysis

## The Core Question

How do we efficiently hydrate forked mainnet state for DeepBook without running a full node?

---

## What Objects Do We Actually Need?

Before choosing an approach, let's enumerate the minimal object set:

### DeepBook V3 Core Objects

| Object Type | Description | Frequency of Change |
|------------|-------------|---------------------|
| **Packages** | DeepBook V3 Move code | Rare (upgrades only) |
| **Pool Objects** | Order books (SUI/USDC, etc.) | Every trade |
| **BalanceManager** | User balance tracking | Every deposit/withdraw |
| **State Object** | Global protocol state | Infrequent |

### Dependencies

| Dependency | What We Need |
|-----------|--------------|
| Coin metadata | USDC, SUI, WAL type definitions |
| Price oracles | If DeepBook uses external oracles |
| Sui Framework | 0x1, 0x2, 0x3 (sui-sandbox includes these) |

### What We DON'T Need

- User coin objects (we mint fake ones in sandbox)
- Historical transactions (we only need current state)
- Non-DeepBook objects (99.9% of chain state)

**Estimated object count**: ~100-1000 objects (pools + state), not millions

---

## Approach Comparison

### Approach 1: Snowflake Query (Recommended for Prototype)

**How it works:**
1. Query Snowflake for DeepBook-related objects by package/type
2. Export object data (ID, version, contents)
3. Load into sui-sandbox as initial state

**Pros:**
- You have access and expertise
- Can fine-tune exactly what's needed
- Fast iteration (query → test → adjust)
- Can validate E2E in hours, not days

**Cons:**
- Not generalizable (others don't have Snowflake)
- Need to know exact object types to query

**Snowflake Query Strategy:**
```sql
-- Find all objects created/modified by DeepBook packages
SELECT
    object_id,
    version,
    object_type,
    object_content  -- or however content is stored
FROM object_state  -- or equivalent table
WHERE object_type LIKE '%deepbook%'
   OR creator_package = '0xdee9...'  -- DeepBook V3 package
```

---

### Approach 2: Formal Snapshot + Filter

**How it works:**
1. Download formal snapshot from `formal-snapshot.mainnet.sui.io`
2. Use `sui-indexer-alt-consistent-store restore` with `--pipeline object_by_type`
3. Filter for DeepBook types during or after restore
4. Export filtered RocksDB subset

**Pros:**
- Replicable by anyone
- Uses official Sui tooling
- Can be automated/scripted

**Cons:**
- Heavy compute (8 CPU, 32GB RAM)
- Slower iteration cycle
- Formal snapshots only go back 60 days (per Ashok's message)
- Need to understand snapshot format

**Key Question:** Can we filter DURING restore, or must we restore everything then filter?

Looking at the restore command:
```bash
sui-indexer-alt-consistent-store restore \
  --pipeline object_by_type \  # <-- This might help filter
  ...
```

The `object_by_type` pipeline could let us query only DeepBook types, but it doesn't store object contents - just references.

---

### Approach 3: Direct RPC/gRPC Queries

**How it works:**
1. Query fullnode RPC for specific objects by ID
2. Use gRPC streaming to keep them updated

**Pros:**
- Simple, no infrastructure needed
- Real-time updates possible

**Cons:**
- Need to know object IDs upfront
- Rate limits on public endpoints
- Doesn't give you discovery (what objects exist?)

**Hybrid potential:** Use Snowflake to discover object IDs, then RPC to fetch contents.

---

### Approach 4: sui-sandbox Built-in Forking

**How it works:**
sui-sandbox already has state forking capabilities (see `fork_state` example)

From the README:
> "Fork real mainnet state and deploy your own contracts against it"
> Fetches actual packages and objects from mainnet via gRPC

**Pros:**
- Already implemented!
- Handles version management
- Caches fetched objects

**Cons:**
- Requires gRPC endpoint access
- Fetches on-demand (cold start latency)
- May hit rate limits for many objects

---

## Recommended Strategy: Layered Approach

### Phase 1: Validate with Snowflake (This Week)

Goal: Prove the E2E flow works

1. Query Snowflake for DeepBook object inventory
2. Export minimal object set (pools, state, packages)
3. Load into sui-sandbox
4. Execute test swap
5. Document exact objects needed

### Phase 2: Use sui-sandbox Native Forking

Goal: Leverage existing tooling

1. Configure sui-sandbox with gRPC endpoint
2. Let it fetch DeepBook objects on-demand
3. Cache fetched objects to local storage
4. Benchmark cold start vs warm start

### Phase 3: Build Generalizable Snapshot Tool (Future)

Goal: Enable anyone to replicate

1. Document object discovery process
2. Script formal snapshot filtering
3. Or: publish pre-filtered DeepBook state snapshots

---

## Snowflake Discovery Queries

### Find DeepBook-related tables/objects

```sql
-- What DeepBook-related data exists?
SELECT DISTINCT object_type
FROM <object_table>
WHERE object_type ILIKE '%deepbook%'
   OR object_type ILIKE '%pool%'
LIMIT 100;
```

### Get Pool Objects

```sql
-- DeepBook V3 pools
SELECT
    object_id,
    object_version,
    object_digest,
    object_content
FROM <object_table>
WHERE object_type = '0xdee9...::pool::Pool<...>'  -- exact type TBD
ORDER BY object_version DESC;
```

### Get Package Objects

```sql
-- DeepBook packages (should be few)
SELECT *
FROM <package_table>
WHERE package_id = '0xdee9...'
   OR package_id IN (SELECT dependency FROM package_deps WHERE ...)
```

---

## Questions to Answer via Snowflake

1. **How many Pool objects exist?** (expect: 10-50)
2. **What are the exact type signatures?** (for filtering)
3. **What's the total data size?** (expect: < 100MB)
4. **What dependencies do pools have?** (dynamic fields, etc.)

---

## Next Steps

1. [ ] Run discovery queries in Snowflake
2. [ ] Identify exact DeepBook object types
3. [ ] Export sample pool object with full content
4. [ ] Test loading into sui-sandbox
5. [ ] Document findings for Phase 2/3
