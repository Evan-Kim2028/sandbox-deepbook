#!/usr/bin/env python3
"""
Export complete DeepBook SUI/USDC state at checkpoint 240M.

All required slices have been verified to exist:
- 10 asks leaf slices
- 10 bids leaf slices
- 2 inner nodes
- 2 pool objects (Pool Wrapper + PoolInner)

Total: 24 objects
"""

# Target checkpoint
TARGET_CHECKPOINT = 240_000_000

# Timestamp range for efficient queries
TIMESTAMP_MIN = 1_765_000_000_000
TIMESTAMP_MAX = 1_770_000_000_000

# Database table
OBJECT_TABLE = "analytics_db_v2.CHAINDATA_MAINNET.OBJECT"

# Known object IDs
POOL_WRAPPER_ID = "0xe05dafb5133bcffb8d59f4e12465dc0e9faeaa05e3e342a08fe135800e3e4407"
POOL_INNER_ID = "0x5c44ceb4c4e8ebb76813c729f8681a449ed1831129ac6e1cf966c7fcefe7dddb"
ASKS_BIGVECTOR_ID = "0x5f8f0e3a2728a161e529ecacdfdface88b2fa669279aa699afd5d6b462c68466"
BIDS_BIGVECTOR_ID = "0x090a8eae3204c76e36eebf3440cbde577e062953391760c37c363530fc1de246"

# Inner node object IDs (verified)
ASKS_INNER_ID = "0xfe76d7120e2e1509b0ffa2113869955a493e75a52127e26a8952e8cd67415407"
BIDS_INNER_ID = "0xf33eb78a2218d83542c9db0192f698341dfa712f6ab9caf8712a1a973c162a8d"

# Asks leaf slice OBJECT_IDs (verified to exist at checkpoint 240M)
ASKS_SLICES = {
    382: "0x548774808ab28e348eb240383e0a4584b6a604f42dc0ba74fc25c83809a9d767",
    1412299: "0xfc08a74d515c4a8616270fc4dcea1b750eec6ea7b5c3eeeacb4c5b9e4defb7ba",
    1775705: "0xa48c26442894582fefa4e9a63d29c49553ff6f3c472172060143284a9b23a081",
    3157423: "0x92e5f96ab464ea1d94e8d29046807755eba794c09d884b479a471999bf14abb4",
    3627837: "0xfbc747771b6cb43370cf477ce0d2c8262454b2e1630d87f78a2f3a545feab57c",
    3712663: "0xf58a4cfc6c31eaf3715a4f9658b8b20f7a90a25938f597ba5eb251abe4806b0f",
    3727881: "0xf92786338592723cf511614a96843ca4c8f98cbd843f0e8f988f99201ee06ab5",  # Verified
    3727913: "0x0e30a23a6c1b89db048890b9750f6f47279e84af2a2ed78600cc4dfd70f80e31",  # Verified
    3727935: "0xb8802099f4330854d3838bdb765d64ef804366571cbf00ce4b7d400c9c7ea281",  # From earlier discovery
    3765078: "0x875a9e63eefde811d5f7c28e305040d5debf68ad27fa7f90d8a9e97d8017a422",  # Verified
}

# Bids leaf slice OBJECT_IDs (all verified at checkpoint 240M)
BIDS_SLICES = {
    102: "0x86b20501868938b9bc10bc695eddb80a0994a1298409286d26da80bf17347f46",
    619149: "0xe7aa909d63ce0a02d7072a0e50c8d70ef21408e9c74b0582b8ee3c413a19ad92",
    1371682: "0xb8c353b637d899962f8b66e14e19f58875c66a6ddc35349c11d78791e402b2d4",
    1499703: "0xda3a6867db53619d97abe99edf75aa8d16e7b1e9aba47560b06d065164ef1c51",
    1504293: "0x8fdc5bd9931acb2164fad22fc967604b66b69542c75a8879ef3bf1fa96f3d73f",
    1506746: "0x50a600e02a2638611f9f3b2cb5642f634cfe9259411ae211946784a5e12284f7",  # Verified
    1507269: "0x8053cd24af4eebba681b6bcf8a36b10b2908f3bb72b1d835e4b64104c357ea85",  # Verified
    1507270: "0x6ae3718650cd7a8b0303505e11ca62471926ef0ed1be4ba1256eea3b20c81416",  # Verified
    1508709: "0x3983595688e461415d8dae0c78b73cf45ec76c1f0647255820fc8d829c3240c0",  # Verified
    1519379: "0x6ff66e40c5431ef4a525c59dc399b2f8b843a69a5e1dff17792f2ece6cfac5cc",  # Verified
}


def get_all_object_ids():
    """Return all object IDs we need to fetch."""
    ids = [
        POOL_WRAPPER_ID,
        POOL_INNER_ID,
        ASKS_INNER_ID,
        BIDS_INNER_ID,
    ]
    ids.extend(ASKS_SLICES.values())
    ids.extend(BIDS_SLICES.values())
    # Remove duplicates while preserving order
    seen = set()
    unique = []
    for id in ids:
        if id not in seen:
            seen.add(id)
            unique.append(id)
    return unique


def generate_export_query():
    """Generate the query to export all objects."""
    all_ids = get_all_object_ids()
    id_list = ",\n        ".join([f"'{id}'" for id in all_ids])

    return f"""
-- Export all DeepBook SUI/USDC state objects at checkpoint {TARGET_CHECKPOINT}
-- Total objects: {len(all_ids)}

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


def generate_batch_queries(batch_size=5):
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
        queries.append(query)

    return queries


if __name__ == "__main__":
    print("=" * 80)
    print(f"DeepBook SUI/USDC State Export at Checkpoint {TARGET_CHECKPOINT:,}")
    print("=" * 80)
    print()

    all_ids = get_all_object_ids()
    print(f"Total objects to fetch: {len(all_ids)}")
    print(f"  - Pool objects: 2")
    print(f"  - Inner nodes: 2")
    print(f"  - Asks leaf slices: {len(ASKS_SLICES)}")
    print(f"  - Bids leaf slices: {len(BIDS_SLICES)}")
    print()

    print("=" * 80)
    print("SINGLE QUERY (may timeout)")
    print("=" * 80)
    print(generate_export_query())

    print()
    print("=" * 80)
    print("BATCH QUERIES (more reliable)")
    print("=" * 80)
    for i, query in enumerate(generate_batch_queries(5), 1):
        print(f"\n-- === BATCH {i} ===")
        print(query)
