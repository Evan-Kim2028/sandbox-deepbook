#!/usr/bin/env python3
"""
Export complete DeepBook WAL/USDC state at checkpoint 240M.

Object hierarchy:
- Pool Wrapper -> PoolInner (dynamic field name=1)
- PoolInner contains:
  - asks BigVector (depth=1, root_id=257580, length=90)
  - bids BigVector (depth=0, root_id=4, length=39)

Total objects: 6
  - Pool Wrapper (1)
  - PoolInner (1)
  - Asks inner node (1)
  - Asks leaf slices (2)
  - Bids leaf slices (1) - depth=0 means no inner node needed

All object IDs VERIFIED from Snowflake at checkpoint 240M.
"""

# Target checkpoint
TARGET_CHECKPOINT = 240_000_000

# Database table
OBJECT_TABLE = "analytics_db_v2.CHAINDATA_MAINNET.OBJECT"

# ============================================================================
# WAL/USDC Pool Object IDs (All Verified)
# ============================================================================

POOL_WRAPPER_ID = "0x56a1c985c1f1123181d6b881714793689321ba24301b3585eec427436eb1c76d"
POOL_INNER_ID = "0xe9b2ac21197de7eb565a8009ab9168e8ae9187aa8ccd4f106f580af562d34ff0"

# BigVector UIDs (from PoolInner)
ASKS_BIGVECTOR_ID = "0x1bf5e16fcfb6c4d293c550bc1333ec7a6ed8323a929bb2db477f63ff0e9b6a4c"  # depth=1, root=257580
BIDS_BIGVECTOR_ID = "0x82ee32196ab12750268815e005fae4c4db23a4272e52610c0c25a8288f05515a"  # depth=0, root=4

# Inner node object ID (only asks has one - depth=1)
# name=257580, vals = [103, 257579]
ASKS_INNER_ID = "0xf2ae25952141a93422722ee188a738848be6b1bb9ed05a514f2e948f4bc81468"

# Leaf slice object IDs
ASKS_SLICES = {
    103: "0x46674c022703ad24f8d3f93798d116caa20b08fb9123594577ae590a9037aa91",     # 49 orders at cp 239999994
    257579: "0xdeab90c7f2afb1f922da2c0f1adae32077e93cdb1188d7709b0fbe2d834c0c82",  # 41 orders at cp 239856184
}

# Bids slices (depth=0, root_id=4 is the direct leaf slice name)
BIDS_SLICES = {
    4: "0x7bf4fbe8cb9945a63e0cbbca7e6bbd5ff7e05f46d80d7715962d5dd74d31e756",       # 39 orders at cp 239999993
}


def get_all_object_ids():
    """Return all object IDs needed for WAL/USDC state."""
    ids = [
        POOL_WRAPPER_ID,
        POOL_INNER_ID,
        ASKS_INNER_ID,
    ]
    ids.extend(ASKS_SLICES.values())
    ids.extend(BIDS_SLICES.values())
    return ids


def generate_export_query():
    """Generate the query to export all objects."""
    all_ids = get_all_object_ids()
    id_list = ",\n        ".join([f"'{id}'" for id in all_ids])

    return f"""
-- Export all DeepBook WAL/USDC state objects at checkpoint {TARGET_CHECKPOINT}
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
        ELSE 4
    END,
    o.OBJECT_ID;
"""


if __name__ == "__main__":
    print("=" * 80)
    print(f"DeepBook WAL/USDC State Export at Checkpoint {TARGET_CHECKPOINT:,}")
    print("=" * 80)
    print()

    all_ids = get_all_object_ids()
    print(f"Total objects to fetch: {len(all_ids)}")
    print(f"  - Pool objects: 2 (Pool Wrapper + PoolInner)")
    print(f"  - Asks inner node: 1 (name=257580, vals=[103, 257579])")
    print(f"  - Asks leaf slices: {len(ASKS_SLICES)}")
    print(f"  - Bids leaf slices: {len(BIDS_SLICES)} (no inner node - depth=0)")
    print()

    print("=" * 80)
    print("EXPORT QUERY")
    print("=" * 80)
    print(generate_export_query())

    print()
    print("=" * 80)
    print("Object ID Summary")
    print("=" * 80)
    print(f"""
Pool Wrapper: {POOL_WRAPPER_ID}
PoolInner:    {POOL_INNER_ID}

Asks BigVector UID: {ASKS_BIGVECTOR_ID}
  depth=1, root_id=257580
  Inner node (name=257580): {ASKS_INNER_ID}
    vals = [103, 257579]
  Leaf slices:
    - name=103:    {ASKS_SLICES[103]}
    - name=257579: {ASKS_SLICES[257579]}

Bids BigVector UID: {BIDS_BIGVECTOR_ID}
  depth=0, root_id=4
  No inner node (depth=0 means direct children)
  Leaf slices:
    - name=4: {BIDS_SLICES[4]}
""")
