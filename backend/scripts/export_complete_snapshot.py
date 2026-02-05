#!/usr/bin/env python3
"""
Export a complete SUI/USDC pool snapshot at checkpoint 149880000 where all BigVector data is present.

This checkpoint was identified as having complete data because:
- Asks inner node vals: [382, 813323] - both exist in Snowflake
- Bids inner node vals: [102, 240375] - both exist in Snowflake

Run with: python scripts/export_complete_snapshot.py
"""

import json
import subprocess
import sys

# Target checkpoint where data is complete
TARGET_CHECKPOINT = 149880000
OUTPUT_FILE = "data/sui_usdc_state_cp149880000.jsonl"

# Object IDs
POOL_WRAPPER_ID = "0xe05dafb5133bcffb8d59f4e12465dc0e9faeaa05e3e342a08fe135800e3e4407"
POOL_INNER_UID = "0x50997b5f1f6401674d3d881a61e09a71776ee19cd8b83114a0a21b3a82f130b5"
ASKS_BIGVECTOR_UID = "0x5f8f0e3a2728a161e529ecacdfdface88b2fa669279aa699afd5d6b462c68466"
BIDS_BIGVECTOR_UID = "0x090a8eae3204c76e36eebf3440cbde577e062953391760c37c363530fc1de246"

def run_snowflake_query(query: str) -> list:
    """Run a Snowflake query using the Snowflake CLI or MCP tool."""
    # For now, we'll print the query and manually run it
    # In production, this would use snowflake-connector-python
    print(f"Query to run:\n{query}\n")
    return []

def export_pool_wrapper():
    """Export Pool wrapper object."""
    query = f"""
    SELECT
        OBJECT_ID as object_id,
        VERSION as version,
        OBJECT_JSON as object_json,
        STRUCT_TAG as object_type,
        OWNER_ADDRESS as owner_address,
        CHECKPOINT as checkpoint
    FROM PIPELINE_V2_GROOT_DB.PIPELINE_V2_GROOT_SCHEMA.OBJECT_PARQUET2 o
    WHERE o.OBJECT_ID = '{POOL_WRAPPER_ID}'
    AND o.CHECKPOINT BETWEEN {TARGET_CHECKPOINT - 100000} AND {TARGET_CHECKPOINT}
    AND o.OBJECT_JSON IS NOT NULL
    ORDER BY CHECKPOINT DESC
    LIMIT 1
    """
    return query

def export_pool_inner():
    """Export PoolInner (dynamic field of Pool)."""
    query = f"""
    SELECT
        OBJECT_ID as object_id,
        VERSION as version,
        OBJECT_JSON as object_json,
        STRUCT_TAG as object_type,
        OWNER_ADDRESS as owner_address,
        CHECKPOINT as checkpoint
    FROM PIPELINE_V2_GROOT_DB.PIPELINE_V2_GROOT_SCHEMA.OBJECT_PARQUET2 o
    WHERE o.OWNER_ADDRESS = '{POOL_INNER_UID}'
    AND o.STRUCT_TAG LIKE '%PoolInner%'
    AND o.CHECKPOINT BETWEEN {TARGET_CHECKPOINT - 100000} AND {TARGET_CHECKPOINT}
    AND o.OBJECT_JSON IS NOT NULL
    ORDER BY CHECKPOINT DESC
    LIMIT 1
    """
    return query

def export_bigvector_slices(bigvector_uid: str, name: str):
    """Export all slices for a BigVector."""
    query = f"""
    SELECT
        OBJECT_ID as object_id,
        VERSION as version,
        OBJECT_JSON as object_json,
        STRUCT_TAG as object_type,
        OWNER_ADDRESS as owner_address,
        CHECKPOINT as checkpoint
    FROM PIPELINE_V2_GROOT_DB.PIPELINE_V2_GROOT_SCHEMA.OBJECT_PARQUET2 o
    WHERE o.OWNER_ADDRESS = '{bigvector_uid}'
    AND o.CHECKPOINT BETWEEN {TARGET_CHECKPOINT - 100000} AND {TARGET_CHECKPOINT}
    AND o.STRUCT_TAG LIKE '%big_vector::Slice%'
    AND o.OBJECT_JSON IS NOT NULL
    QUALIFY ROW_NUMBER() OVER (PARTITION BY o.OBJECT_ID ORDER BY o.CHECKPOINT DESC) = 1
    ORDER BY OBJECT_ID
    """
    return query

def main():
    print(f"=== Export Complete Snapshot at Checkpoint {TARGET_CHECKPOINT} ===\n")

    print("1. Pool Wrapper Query:")
    print(export_pool_wrapper())

    print("\n2. PoolInner Query:")
    print(export_pool_inner())

    print("\n3. Asks BigVector Slices Query:")
    print(export_bigvector_slices(ASKS_BIGVECTOR_UID, "asks"))

    print("\n4. Bids BigVector Slices Query:")
    print(export_bigvector_slices(BIDS_BIGVECTOR_UID, "bids"))

    print(f"\nOutput file: {OUTPUT_FILE}")
    print("\nRun these queries in Snowflake and combine results into the JSONL file.")
    print("Each line should be a JSON object with: object_id, version, object_json, object_type, owner_address, checkpoint")

if __name__ == "__main__":
    main()
