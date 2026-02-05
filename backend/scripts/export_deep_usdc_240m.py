#!/usr/bin/env python3
"""
Export complete DeepBook DEEP/USDC state at checkpoint 240M.

Object hierarchy:
- Pool Wrapper -> PoolInner (dynamic field name=1)
- PoolInner contains:
  - asks BigVector (depth=1, root_id=325886, length=104)
  - bids BigVector (depth=1, root_id=326657, length=131)
- Inner nodes (Slice<u64>) contain vals array pointing to leaf slice names
- Leaf nodes (Slice<Order>) contain actual order data

Total objects needed: 8
  - Pool Wrapper (1)
  - PoolInner (1)
  - Inner nodes (2)
  - Asks leaf slices (2)
  - Bids leaf slices (4)

Status: COMPLETE - All object IDs verified
"""

# Target checkpoint
TARGET_CHECKPOINT = 240_000_000

# Timestamp range for efficient queries (checkpoint 240M is around 1769-1770 trillion ms)
TIMESTAMP_MIN = 1_765_000_000_000
TIMESTAMP_MAX = 1_770_500_000_000

# Database table
OBJECT_TABLE = "analytics_db_v2.CHAINDATA_MAINNET.OBJECT"

# ============================================================================
# DEEP/USDC Pool Object IDs (Verified)
# ============================================================================

POOL_WRAPPER_ID = "0xf948981b806057580f91622417534f491da5f61aeaf33d0ed8e69fd5691c95ce"
POOL_INNER_ID = "0xe2d83da90575a47ccb7e829804896e28107b519c356e6872fd3f36ea6ae32418"

# BigVector UIDs (extracted from PoolInner)
ASKS_BIGVECTOR_ID = "0x0f9d6fc9de7a0ee0dd98f7326619cd5ff74cc0bc6485cce80014f766e437c4ae"
BIDS_BIGVECTOR_ID = "0xd1fcd1d0a554150fa097508eabcd76f6dbb0d2ce4fdfeffb2f6a4469ac81fd42"

# Inner node object IDs (verified)
# Asks inner node (root, name=325886): vals = [1141, 356793]
ASKS_INNER_ID = "0xb0a594baac95d931ad67131cb4f21a4fdd28975f8d8076bc561b60b306f870b2"
# Bids inner node (root, name=326657): vals = [294, 326658, 742528, 781892]
BIDS_INNER_ID = "0xa9462f22b79745b1eca735af2eaaa76b75750d3125e4e4793f4de5723d8465fc"

# ============================================================================
# Leaf Slice Object IDs (All Verified)
# ============================================================================

# Asks leaf slices (from inner node vals: [1141, 356793])
ASKS_SLICES = {
    1141: "0x7404704547b4fca6ef2ea15de83f8f8b787b2dee04ec3572e2e17fca42a5abf0",    # Verified at cp 239999999
    356793: "0xc6e846eda2d318b23c4876704f2f1ca97e34c56984f15ae085574ababd97da76",  # Verified at cp 238619361
}

# Bids leaf slices (from inner node vals: [294, 326658, 742528, 781892])
BIDS_SLICES = {
    294: "0xfe47e49dcefb86bfb8ad0846293a6ceec5fe3b901f6efe015fe99e118d1b01ec",     # Verified at cp 70248863
    326658: "0xfaef9a7a931640dcdb3b17809b9d69bd8935dc5873bea8e6dfb0b71aca3fbc17",  # Verified at cp 223966351
    742528: "0x436d05d0d57cf7532faaf9e812555543f78051f67e9344eeb435f383c2c16ca7",  # Verified at cp 238703789
    781892: "0x1d99c7ec6d7674badc59cfb8ded8683fccbf1cd8e796a91e8cbdf418f6013ba0",  # Verified at cp 239999973
}


def get_all_object_ids():
    """Return all object IDs needed for DEEP/USDC state."""
    ids = [
        POOL_WRAPPER_ID,
        POOL_INNER_ID,
        ASKS_INNER_ID,
        BIDS_INNER_ID,
    ]
    ids.extend(ASKS_SLICES.values())
    ids.extend(BIDS_SLICES.values())
    return ids


def generate_export_query():
    """Generate the query to export all objects."""
    all_ids = get_all_object_ids()
    id_list = ",\n        ".join([f"'{id}'" for id in all_ids])

    return f"""
-- Export all DeepBook DEEP/USDC state objects at checkpoint {TARGET_CHECKPOINT}
-- Total objects: {len(all_ids)}

WITH target_objects AS (
    SELECT OBJECT_ID, MAX(VERSION) as max_version
    FROM {OBJECT_TABLE}
    WHERE OBJECT_ID IN (
        {id_list}
    )
    AND CHECKPOINT <= {TARGET_CHECKPOINT}
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
WHERE o.OBJECT_JSON IS NOT NULL
ORDER BY
    CASE
        WHEN o.OBJECT_ID = '{POOL_WRAPPER_ID}' THEN 1
        WHEN o.OBJECT_ID = '{POOL_INNER_ID}' THEN 2
        WHEN o.OBJECT_ID = '{ASKS_INNER_ID}' THEN 3
        WHEN o.OBJECT_ID = '{BIDS_INNER_ID}' THEN 4
        ELSE 5
    END,
    o.OBJECT_ID;
"""


def generate_batch_queries(batch_size=4):
    """Generate smaller batch queries for more reliable execution."""
    all_ids = get_all_object_ids()
    queries = []

    for i in range(0, len(all_ids), batch_size):
        batch = all_ids[i:i + batch_size]
        id_list = ", ".join([f"'{id}'" for id in batch])

        query = f"""
-- Batch {i // batch_size + 1}: Objects {i + 1} to {min(i + batch_size, len(all_ids))}
WITH target_objects AS (
    SELECT OBJECT_ID, MAX(VERSION) as max_version
    FROM {OBJECT_TABLE}
    WHERE OBJECT_ID IN ({id_list})
    AND CHECKPOINT <= {TARGET_CHECKPOINT}
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
        queries.append(query)

    return queries


if __name__ == "__main__":
    print("=" * 80)
    print(f"DeepBook DEEP/USDC State Export at Checkpoint {TARGET_CHECKPOINT:,}")
    print("=" * 80)
    print()

    all_ids = get_all_object_ids()
    print(f"Total objects to fetch: {len(all_ids)}")
    print(f"  - Pool objects: 2 (Pool Wrapper + PoolInner)")
    print(f"  - Inner nodes: 2 (Asks root + Bids root)")
    print(f"  - Asks leaf slices: {len(ASKS_SLICES)}")
    print(f"  - Bids leaf slices: {len(BIDS_SLICES)}")
    print()

    print("=" * 80)
    print("SINGLE QUERY (may timeout on large datasets)")
    print("=" * 80)
    print(generate_export_query())

    print()
    print("=" * 80)
    print("BATCH QUERIES (more reliable)")
    print("=" * 80)
    for i, query in enumerate(generate_batch_queries(4), 1):
        print(f"\n-- === BATCH {i} ===")
        print(query)

    print()
    print("=" * 80)
    print("Object ID Summary")
    print("=" * 80)
    print(f"""
Pool Wrapper: {POOL_WRAPPER_ID}
PoolInner:    {POOL_INNER_ID}

Asks BigVector UID: {ASKS_BIGVECTOR_ID}
  Inner node (name=325886): {ASKS_INNER_ID}
  Leaf slices:
    - name=1141:   {ASKS_SLICES[1141]}
    - name=356793: {ASKS_SLICES[356793]}

Bids BigVector UID: {BIDS_BIGVECTOR_ID}
  Inner node (name=326657): {BIDS_INNER_ID}
  Leaf slices:
    - name=294:    {BIDS_SLICES[294]}
    - name=326658: {BIDS_SLICES[326658]}
    - name=742528: {BIDS_SLICES[742528]}
    - name=781892: {BIDS_SLICES[781892]}
""")
