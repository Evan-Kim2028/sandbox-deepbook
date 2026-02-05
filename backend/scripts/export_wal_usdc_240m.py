#!/usr/bin/env python3
"""
Export complete DeepBook WAL/USDC state at checkpoint 240M.

Object hierarchy:
- Pool Wrapper -> PoolInner (dynamic field name=1)
- PoolInner contains:
  - asks BigVector (depth=1, root_id=257580, length=90)
  - bids BigVector (depth=0, root_id=4, length=39)
- For depth=0 (bids): No inner node, leaf slices are direct children
- For depth=1 (asks): Inner node contains vals array pointing to leaf slice names

Status: PARTIAL - Need to discover inner node and leaf slice object IDs
"""

# Target checkpoint
TARGET_CHECKPOINT = 240_000_000

# Timestamp range for efficient queries
TIMESTAMP_MIN = 1_765_000_000_000
TIMESTAMP_MAX = 1_770_500_000_000

# Database table
OBJECT_TABLE = "analytics_db_v2.CHAINDATA_MAINNET.OBJECT"

# ============================================================================
# WAL/USDC Pool Object IDs (Verified)
# ============================================================================

POOL_WRAPPER_ID = "0x56a1c985c1f1123181d6b881714793689321ba24301b3585eec427436eb1c76d"
POOL_INNER_ID = "0xe9b2ac21197de7eb565a8009ab9168e8ae9187aa8ccd4f106f580af562d34ff0"

# BigVector UIDs (extracted from PoolInner)
ASKS_BIGVECTOR_ID = "0x1bf5e16fcfb6c4d293c550bc1333ec7a6ed8323a929bb2db477f63ff0e9b6a4c"
BIDS_BIGVECTOR_ID = "0x82ee32196ab12750268815e005fae4c4db23a4272e52610c0c25a8288f05515a"

# BigVector structure from PoolInner:
# asks: depth=1, root_id=257580, length=90
# bids: depth=0, root_id=4, length=39

# Inner node object IDs (need to discover)
# Asks inner node (name=257580) - depth=1 means one level of inner nodes
ASKS_INNER_ID = None  # TODO: Query to find

# For depth=0 bids, there's no inner node - leaf slices are direct children
BIDS_INNER_ID = None  # Not applicable (depth=0)

# Leaf slice object IDs (need to discover)
ASKS_SLICES = {}  # TODO: Discover from inner node vals
BIDS_SLICES = {
    # For depth=0, root_id (4) IS the leaf slice name
    4: None,  # TODO: Query to find
}


def generate_discovery_queries():
    """Generate queries to discover missing object IDs."""
    queries = []

    # Query to find asks inner node (Slice<u64> with name=257580)
    queries.append(f"""
-- Step 1: Find asks inner node (Slice<u64> with name=257580)
-- Owner = Asks BigVector UID
SELECT OBJECT_ID, CHECKPOINT, TIMESTAMP_MS, OBJECT_JSON
FROM {OBJECT_TABLE}
WHERE OWNER_ADDRESS = '{ASKS_BIGVECTOR_ID}'
AND STRUCT_TAG LIKE '%Slice<u64>%'
AND CHECKPOINT <= {TARGET_CHECKPOINT}
ORDER BY CHECKPOINT DESC
LIMIT 5;
""")

    # Query to find asks leaf slices (Slice<Order>)
    queries.append(f"""
-- Step 2: Find asks leaf slices (Slice<Order>)
-- Owner = Asks BigVector UID, struct_tag contains Order
SELECT OBJECT_ID, CHECKPOINT, TIMESTAMP_MS, LEFT(OBJECT_JSON, 300) as JSON_PREVIEW
FROM {OBJECT_TABLE}
WHERE OWNER_ADDRESS = '{ASKS_BIGVECTOR_ID}'
AND STRUCT_TAG LIKE '%Order%'
AND CHECKPOINT <= {TARGET_CHECKPOINT}
ORDER BY CHECKPOINT DESC
LIMIT 20;
""")

    # Query to find bids leaf slices (depth=0 means no inner node)
    queries.append(f"""
-- Step 3: Find bids leaf slices (Slice<Order>)
-- For depth=0, leaf slices are direct children of BigVector UID
-- Owner = Bids BigVector UID
SELECT OBJECT_ID, CHECKPOINT, TIMESTAMP_MS, LEFT(OBJECT_JSON, 300) as JSON_PREVIEW
FROM {OBJECT_TABLE}
WHERE OWNER_ADDRESS = '{BIDS_BIGVECTOR_ID}'
AND STRUCT_TAG LIKE '%Order%'
AND CHECKPOINT <= {TARGET_CHECKPOINT}
ORDER BY CHECKPOINT DESC
LIMIT 20;
""")

    return queries


def get_all_object_ids():
    """Return all known object IDs."""
    ids = [
        POOL_WRAPPER_ID,
        POOL_INNER_ID,
    ]
    if ASKS_INNER_ID:
        ids.append(ASKS_INNER_ID)
    for slice_id in ASKS_SLICES.values():
        if slice_id:
            ids.append(slice_id)
    for slice_id in BIDS_SLICES.values():
        if slice_id:
            ids.append(slice_id)
    return ids


def generate_export_query():
    """Generate the query to export all known objects."""
    all_ids = get_all_object_ids()
    id_list = ",\n        ".join([f"'{id}'" for id in all_ids])

    return f"""
-- Export all DeepBook WAL/USDC state objects at checkpoint {TARGET_CHECKPOINT}
-- Total objects: {len(all_ids)} (known)
-- Note: Most object IDs need to be discovered first

WITH target_objects AS (
    SELECT OBJECT_ID, MAX(VERSION) as max_version
    FROM {OBJECT_TABLE}
    WHERE OBJECT_ID IN (
        {id_list}
    )
    AND CHECKPOINT <= {TARGET_CHECKPOINT}
    AND TIMESTAMP_MS BETWEEN {TIMESTAMP_MIN} AND {TIMESTAMP_MAX}
    GROUP BY OBJECT_ID
)
SELECT
    o.OBJECT_ID,
    o.VERSION,
    o.CHECKPOINT,
    o.STRUCT_TAG,
    o.OWNER_ADDRESS,
    o.OBJECT_JSON
FROM {OBJECT_TABLE} o
INNER JOIN target_objects t
    ON o.OBJECT_ID = t.OBJECT_ID
    AND o.VERSION = t.max_version
WHERE o.OBJECT_JSON IS NOT NULL;
"""


if __name__ == "__main__":
    print("=" * 80)
    print(f"DeepBook WAL/USDC State Export at Checkpoint {TARGET_CHECKPOINT:,}")
    print("=" * 80)
    print()

    print("Object Discovery Status:")
    print(f"  - Pool Wrapper: ✓ Known ({POOL_WRAPPER_ID})")
    print(f"  - PoolInner: ✓ Known ({POOL_INNER_ID})")
    print(f"  - Asks inner node: {'✓ Known' if ASKS_INNER_ID else '✗ Need to discover'}")
    print(f"  - Asks leaf slices: {len([v for v in ASKS_SLICES.values() if v])}/{len(ASKS_SLICES) or '?'} known")
    print(f"  - Bids leaf slices: {len([v for v in BIDS_SLICES.values() if v])}/{len(BIDS_SLICES) or '?'} known")
    print()

    print("=" * 80)
    print("DISCOVERY QUERIES (run these to find object IDs)")
    print("=" * 80)
    for i, query in enumerate(generate_discovery_queries(), 1):
        print(f"\n-- Query {i}")
        print(query)

    print()
    print("=" * 80)
    print("BigVector Structure (from PoolInner):")
    print("=" * 80)
    print(f"""
WAL/USDC Asks (depth=1, root_id=257580, length=90):
  BigVector UID: {ASKS_BIGVECTOR_ID}

  For depth=1:
  - Need to find inner node with name=257580
  - Inner node's vals array contains leaf slice names
  - Then fetch each leaf slice

WAL/USDC Bids (depth=0, root_id=4, length=39):
  BigVector UID: {BIDS_BIGVECTOR_ID}

  For depth=0:
  - No inner node needed
  - root_id (4) is directly the first leaf slice name
  - Leaf slices are direct children of BigVector UID
""")
