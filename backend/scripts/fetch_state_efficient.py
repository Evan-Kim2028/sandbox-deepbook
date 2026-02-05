#!/usr/bin/env python3
"""
Efficient Snowflake queries to fetch DeepBook V3 state at a target checkpoint.

IMPORTANT: Use analytics_db_v2.CHAINDATA_MAINNET.OBJECT, NOT OBJECT_PARQUET2!
- OBJECT_PARQUET2: Only has data up to checkpoint ~114M
- CHAINDATA_MAINNET.OBJECT: Has data up to checkpoint 241M+ with OBJECT_JSON

Key optimizations:
1. Use aggregate MAX(VERSION) instead of QUALIFY ROW_NUMBER (much faster)
2. Use TIMESTAMP_MS filtering for partition pruning
3. Two-step approach: find max versions first, then fetch objects
4. Batch queries where possible

Usage:
    python scripts/fetch_state_efficient.py

This generates SQL queries to run in Snowflake. Execute them and save results to JSONL.
"""

import json
from dataclasses import dataclass
from typing import List, Optional

# Target checkpoint
TARGET_CHECKPOINT = 240_000_000

# Timestamp range for checkpoint 240M
# Checkpoint 240M is roughly late January 2026
# Use TIMESTAMP_MS filter for partition pruning
# 1 checkpoint ≈ 0.5 seconds, so 240M checkpoints ≈ 120M seconds from genesis
# Genesis was ~May 2023, so 240M is roughly Jan-Feb 2026
TIMESTAMP_MIN = 1_735_689_600_000  # Jan 1, 2025
TIMESTAMP_MAX = 1_770_000_000_000  # ~Feb 2026

# The correct table to use (has recent data with OBJECT_JSON)
OBJECT_TABLE = "analytics_db_v2.CHAINDATA_MAINNET.OBJECT"

# Known object IDs for SUI/USDC pool
POOL_WRAPPER_ID = "0xe05dafb5133bcffb8d59f4e12465dc0e9faeaa05e3e342a08fe135800e3e4407"
POOL_INNER_ID = "0x5c44ceb4c4e8ebb76813c729f8681a449ed1831129ac6e1cf966c7fcefe7dddb"
POOL_INNER_PARENT = "0x50997b5f1f6401674d3d881a61e09a71776ee19cd8b83114a0a21b3a82f130b5"
ASKS_BIGVECTOR_ID = "0x5f8f0e3a2728a161e529ecacdfdface88b2fa669279aa699afd5d6b462c68466"
BIDS_BIGVECTOR_ID = "0x090a8eae3204c76e36eebf3440cbde577e062953391760c37c363530fc1de246"


def generate_pool_objects_query() -> str:
    """
    Query to get Pool Wrapper and PoolInner at target checkpoint.
    Uses MAX(VERSION) aggregation instead of QUALIFY ROW_NUMBER.
    """
    return f"""
-- Step 1: Get Pool Wrapper and PoolInner
-- Uses MAX aggregation (fast) instead of QUALIFY ROW_NUMBER (slow)
WITH max_versions AS (
    SELECT
        OBJECT_ID,
        MAX(VERSION) as max_version
    FROM {OBJECT_TABLE}
    WHERE OBJECT_ID IN (
        '{POOL_WRAPPER_ID}',
        '{POOL_INNER_ID}'
    )
    AND CHECKPOINT <= {TARGET_CHECKPOINT}
    AND TIMESTAMP_MS BETWEEN {TIMESTAMP_MIN} AND {TIMESTAMP_MAX}
    GROUP BY OBJECT_ID
)
SELECT
    'pool_object' as obj_type,
    o.OBJECT_ID,
    o.VERSION,
    o.CHECKPOINT,
    o.STRUCT_TAG,
    o.OWNER_ADDRESS,
    o.OBJECT_JSON
FROM {OBJECT_TABLE} o
INNER JOIN max_versions mv
    ON o.OBJECT_ID = mv.OBJECT_ID
    AND o.VERSION = mv.max_version
WHERE o.OBJECT_JSON IS NOT NULL;
"""


def generate_inner_nodes_query() -> str:
    """
    Query to get inner nodes (Slice<u64>) for asks and bids BigVectors.
    These contain the vals array pointing to leaf slices.
    """
    return f"""
-- Step 2: Get Inner Nodes (Slice<u64>) - these tell us which leaf slices to fetch
-- Inner nodes are dynamic fields owned by the BigVector UIDs
WITH max_versions AS (
    SELECT
        OBJECT_ID,
        MAX(VERSION) as max_version
    FROM {OBJECT_TABLE}
    WHERE OWNER_ADDRESS IN (
        '{ASKS_BIGVECTOR_ID}',
        '{BIDS_BIGVECTOR_ID}'
    )
    AND STRUCT_TAG LIKE '%Slice<u64>%'
    AND CHECKPOINT <= {TARGET_CHECKPOINT}
    AND TIMESTAMP_MS BETWEEN {TIMESTAMP_MIN} AND {TIMESTAMP_MAX}
    GROUP BY OBJECT_ID
)
SELECT
    CASE
        WHEN o.OWNER_ADDRESS = '{ASKS_BIGVECTOR_ID}' THEN 'asks_inner'
        ELSE 'bids_inner'
    END as obj_type,
    o.OBJECT_ID,
    o.VERSION,
    o.CHECKPOINT,
    o.STRUCT_TAG,
    o.OWNER_ADDRESS,
    o.OBJECT_JSON
FROM {OBJECT_TABLE} o
INNER JOIN max_versions mv
    ON o.OBJECT_ID = mv.OBJECT_ID
    AND o.VERSION = mv.max_version
WHERE o.OBJECT_JSON IS NOT NULL;
"""


def generate_leaf_slices_query(side: str, bigvector_id: str) -> str:
    """
    Query to get ALL leaf slices (Slice<Order>) for a BigVector.
    We get all slices and filter in code based on inner node vals.
    """
    return f"""
-- Step 3{side[0].upper()}: Get ALL {side} leaf slices (Slice<Order>)
-- We fetch all and filter based on inner node vals
WITH max_versions AS (
    SELECT
        OBJECT_ID,
        MAX(VERSION) as max_version
    FROM {OBJECT_TABLE}
    WHERE OWNER_ADDRESS = '{bigvector_id}'
    AND STRUCT_TAG LIKE '%Slice<0x2c8d603bc51326b8c13cef9dd07031a408a48dddb541963357661df5d3204809::order::Order>%'
    AND CHECKPOINT <= {TARGET_CHECKPOINT}
    AND TIMESTAMP_MS BETWEEN {TIMESTAMP_MIN} AND {TIMESTAMP_MAX}
    GROUP BY OBJECT_ID
)
SELECT
    '{side}_slice' as obj_type,
    o.OBJECT_ID,
    o.VERSION,
    o.CHECKPOINT,
    o.STRUCT_TAG,
    o.OWNER_ADDRESS,
    o.OBJECT_JSON
FROM {OBJECT_TABLE} o
INNER JOIN max_versions mv
    ON o.OBJECT_ID = mv.OBJECT_ID
    AND o.VERSION = mv.max_version
WHERE o.OBJECT_JSON IS NOT NULL;
"""


def generate_combined_query() -> str:
    """
    Single combined query that fetches everything in one go.
    More efficient for network round-trips but may be slower per-query.
    """
    return f"""
-- Combined query: Fetch all DeepBook state objects at checkpoint {TARGET_CHECKPOINT}
-- Uses UNION ALL for efficiency
-- TABLE: {OBJECT_TABLE}

-- Pool objects
WITH pool_max AS (
    SELECT OBJECT_ID, MAX(VERSION) as max_version
    FROM {OBJECT_TABLE}
    WHERE OBJECT_ID IN ('{POOL_WRAPPER_ID}', '{POOL_INNER_ID}')
    AND CHECKPOINT <= {TARGET_CHECKPOINT}
    AND TIMESTAMP_MS BETWEEN {TIMESTAMP_MIN} AND {TIMESTAMP_MAX}
    GROUP BY OBJECT_ID
),
-- Inner nodes (Slice<u64>)
inner_max AS (
    SELECT OBJECT_ID, MAX(VERSION) as max_version
    FROM {OBJECT_TABLE}
    WHERE OWNER_ADDRESS IN ('{ASKS_BIGVECTOR_ID}', '{BIDS_BIGVECTOR_ID}')
    AND STRUCT_TAG LIKE '%Slice<u64>%'
    AND CHECKPOINT <= {TARGET_CHECKPOINT}
    AND TIMESTAMP_MS BETWEEN {TIMESTAMP_MIN} AND {TIMESTAMP_MAX}
    GROUP BY OBJECT_ID
),
-- Asks leaf slices
asks_max AS (
    SELECT OBJECT_ID, MAX(VERSION) as max_version
    FROM {OBJECT_TABLE}
    WHERE OWNER_ADDRESS = '{ASKS_BIGVECTOR_ID}'
    AND STRUCT_TAG LIKE '%Slice<0x2c8d603bc51326b8c13cef9dd07031a408a48dddb541963357661df5d3204809::order::Order>%'
    AND CHECKPOINT <= {TARGET_CHECKPOINT}
    AND TIMESTAMP_MS BETWEEN {TIMESTAMP_MIN} AND {TIMESTAMP_MAX}
    GROUP BY OBJECT_ID
),
-- Bids leaf slices
bids_max AS (
    SELECT OBJECT_ID, MAX(VERSION) as max_version
    FROM {OBJECT_TABLE}
    WHERE OWNER_ADDRESS = '{BIDS_BIGVECTOR_ID}'
    AND STRUCT_TAG LIKE '%Slice<0x2c8d603bc51326b8c13cef9dd07031a408a48dddb541963357661df5d3204809::order::Order>%'
    AND CHECKPOINT <= {TARGET_CHECKPOINT}
    AND TIMESTAMP_MS BETWEEN {TIMESTAMP_MIN} AND {TIMESTAMP_MAX}
    GROUP BY OBJECT_ID
)

-- Fetch all objects
SELECT 'pool' as category, o.OBJECT_ID, o.VERSION, o.CHECKPOINT, o.STRUCT_TAG, o.OWNER_ADDRESS, o.OBJECT_JSON
FROM {OBJECT_TABLE} o
JOIN pool_max m ON o.OBJECT_ID = m.OBJECT_ID AND o.VERSION = m.max_version
WHERE o.OBJECT_JSON IS NOT NULL

UNION ALL

SELECT 'inner' as category, o.OBJECT_ID, o.VERSION, o.CHECKPOINT, o.STRUCT_TAG, o.OWNER_ADDRESS, o.OBJECT_JSON
FROM {OBJECT_TABLE} o
JOIN inner_max m ON o.OBJECT_ID = m.OBJECT_ID AND o.VERSION = m.max_version
WHERE o.OBJECT_JSON IS NOT NULL

UNION ALL

SELECT 'asks_leaf' as category, o.OBJECT_ID, o.VERSION, o.CHECKPOINT, o.STRUCT_TAG, o.OWNER_ADDRESS, o.OBJECT_JSON
FROM {OBJECT_TABLE} o
JOIN asks_max m ON o.OBJECT_ID = m.OBJECT_ID AND o.VERSION = m.max_version
WHERE o.OBJECT_JSON IS NOT NULL

UNION ALL

SELECT 'bids_leaf' as category, o.OBJECT_ID, o.VERSION, o.CHECKPOINT, o.STRUCT_TAG, o.OWNER_ADDRESS, o.OBJECT_JSON
FROM {OBJECT_TABLE} o
JOIN bids_max m ON o.OBJECT_ID = m.OBJECT_ID AND o.VERSION = m.max_version
WHERE o.OBJECT_JSON IS NOT NULL;
"""


def generate_discovery_query() -> str:
    """
    Discovery query to find what slices exist and their checkpoints.
    Run this first to understand what data is available.
    """
    return f"""
-- Discovery: What slices exist for asks/bids at checkpoint <= {TARGET_CHECKPOINT}?
-- This helps identify if any slices are missing
-- TABLE: {OBJECT_TABLE}

-- Asks slices summary
SELECT
    'asks' as side,
    COUNT(DISTINCT OBJECT_ID) as slice_count,
    MIN(CHECKPOINT) as min_checkpoint,
    MAX(CHECKPOINT) as max_checkpoint
FROM {OBJECT_TABLE}
WHERE OWNER_ADDRESS = '{ASKS_BIGVECTOR_ID}'
AND STRUCT_TAG LIKE '%Slice<%'
AND CHECKPOINT <= {TARGET_CHECKPOINT}
AND TIMESTAMP_MS BETWEEN {TIMESTAMP_MIN} AND {TIMESTAMP_MAX}

UNION ALL

-- Bids slices summary
SELECT
    'bids' as side,
    COUNT(DISTINCT OBJECT_ID) as slice_count,
    MIN(CHECKPOINT) as min_checkpoint,
    MAX(CHECKPOINT) as max_checkpoint
FROM {OBJECT_TABLE}
WHERE OWNER_ADDRESS = '{BIDS_BIGVECTOR_ID}'
AND STRUCT_TAG LIKE '%Slice<%'
AND CHECKPOINT <= {TARGET_CHECKPOINT}
AND TIMESTAMP_MS BETWEEN {TIMESTAMP_MIN} AND {TIMESTAMP_MAX};
"""


def generate_inner_node_vals_query() -> str:
    """
    Query to extract the vals array from inner nodes.
    This tells us exactly which slice names we need.
    """
    return f"""
-- Extract inner node vals to know which slices are required
-- Run after getting inner nodes to parse the vals array
-- TABLE: {OBJECT_TABLE}

WITH inner_nodes AS (
    SELECT
        OBJECT_ID,
        MAX(VERSION) as max_version
    FROM {OBJECT_TABLE}
    WHERE OWNER_ADDRESS IN ('{ASKS_BIGVECTOR_ID}', '{BIDS_BIGVECTOR_ID}')
    AND STRUCT_TAG LIKE '%Slice<u64>%'
    AND CHECKPOINT <= {TARGET_CHECKPOINT}
    AND TIMESTAMP_MS BETWEEN {TIMESTAMP_MIN} AND {TIMESTAMP_MAX}
    GROUP BY OBJECT_ID
)
SELECT
    CASE
        WHEN o.OWNER_ADDRESS = '{ASKS_BIGVECTOR_ID}' THEN 'asks'
        ELSE 'bids'
    END as side,
    o.OBJECT_ID,
    o.CHECKPOINT,
    -- Extract the vals array from JSON
    OBJECT_JSON:fields:value:fields:vals as vals_array
FROM {OBJECT_TABLE} o
JOIN inner_nodes n ON o.OBJECT_ID = n.OBJECT_ID AND o.VERSION = n.max_version
WHERE o.OBJECT_JSON IS NOT NULL;
"""


def main():
    print("=" * 80)
    print(f"EFFICIENT SNOWFLAKE QUERIES FOR DEEPBOOK STATE AT CHECKPOINT {TARGET_CHECKPOINT:,}")
    print("=" * 80)
    print()
    print("Key optimizations applied:")
    print("  1. MAX(VERSION) aggregation instead of QUALIFY ROW_NUMBER")
    print("  2. TIMESTAMP_MS filtering for partition pruning")
    print("  3. Two-step approach: find versions first, then fetch")
    print("  4. Batch queries with UNION ALL")
    print()

    print("-" * 80)
    print("STEP 0: DISCOVERY - Check what data exists")
    print("-" * 80)
    print(generate_discovery_query())

    print("-" * 80)
    print("STEP 1: GET POOL OBJECTS (Pool Wrapper + PoolInner)")
    print("-" * 80)
    print(generate_pool_objects_query())

    print("-" * 80)
    print("STEP 2: GET INNER NODES (to find required slice names)")
    print("-" * 80)
    print(generate_inner_nodes_query())

    print("-" * 80)
    print("STEP 2.5: EXTRACT VALS ARRAY (to verify required slices)")
    print("-" * 80)
    print(generate_inner_node_vals_query())

    print("-" * 80)
    print("STEP 3A: GET ASKS LEAF SLICES")
    print("-" * 80)
    print(generate_leaf_slices_query("asks", ASKS_BIGVECTOR_ID))

    print("-" * 80)
    print("STEP 3B: GET BIDS LEAF SLICES")
    print("-" * 80)
    print(generate_leaf_slices_query("bids", BIDS_BIGVECTOR_ID))

    print("-" * 80)
    print("ALTERNATIVE: COMBINED SINGLE QUERY")
    print("-" * 80)
    print(generate_combined_query())

    print()
    print("=" * 80)
    print("NEXT STEPS:")
    print("=" * 80)
    print("""
1. Run the DISCOVERY query first to see slice counts
2. Run STEP 2 to get inner nodes and their vals arrays
3. Parse the vals arrays to get required slice names
4. Run STEP 3A/3B to get leaf slices
5. Verify all required slices are present
6. If slices are missing, try an earlier checkpoint
7. Export results to JSONL format for the Rust loader
""")


if __name__ == "__main__":
    main()
