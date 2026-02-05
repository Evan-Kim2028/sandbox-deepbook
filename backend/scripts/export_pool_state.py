#!/usr/bin/env python3
"""
Export DeepBook pool state from Snowflake to JSONL files.
Exports pool wrapper, PoolInner, and all BigVector slices for the orderbook.
"""

import json
import subprocess
import sys
from pathlib import Path

# Pool configurations
POOLS = {
    "wal_usdc": {
        "name": "WAL/USDC",
        "pool_id": "0x56a1c985c1f1123181d6b881714793689321ba24301b3585eec427436eb1c76d",
        "pool_inner_uid": "0xe28eca4e6470c7a326f58eadb0482665b5f0831be0c1a0f8f33a0a998729f0d3",
        "bids_bigvector": "0x82ee32196ab12750268815e005fae4c4db23a4272e52610c0c25a8288f05515a",
        "asks_bigvector": "0x1bf5e16fcfb6c4d293c550bc1333ec7a6ed8323a929bb2db477f63ff0e9b6a4c",
    },
    "deep_usdc": {
        "name": "DEEP/USDC",
        "pool_id": "0xf948981b806057580f91622417534f491da5f61aeaf33d0ed8e69fd5691c95ce",
        "pool_inner_uid": "0xac73b6fd7dfca972f1f583c3b59daa110cb44c9a3419cf697533f87e9e7bb7f4",
        "bids_bigvector": "0xd1fcd1d0a554150fa097508eabcd76f6dbb0d2ce4fdfeffb2f6a4469ac81fd42",
        "asks_bigvector": "0x0f9d6fc9de7a0ee0dd98f7326619cd5ff74cc0bc6485cce80014f766e437c4ae",
    },
    "sui_usdc": {
        "name": "SUI/USDC",
        "pool_id": "0xe05dafb5133bcffb8d59f4e12465dc0e9faeaa05e3e342a08fe135800e3e4407",
        "pool_inner_uid": "0x50997b5f1f6401674d3d881a61e09a71776ee19cd8b83114a0a21b3a82f130b5",
        "bids_bigvector": "0x090a8eae3204c76e36eebf3440cbde577e062953391760c37c363530fc1de246",
        "asks_bigvector": "0x5f8f0e3a2728a161e529ecacdfdface88b2fa669279aa699afd5d6b462c68466",
    },
}

# Target checkpoint for Feb 2, 2026
TARGET_CHECKPOINT = 241056077


def generate_export_query(pool_config: dict) -> str:
    """Generate SQL query to export pool state."""
    return f"""
WITH latest_objects AS (
    SELECT
        o.OBJECT_ID,
        o.OBJECT_TYPE,
        o.VERSION,
        o.OBJECT_JSON,
        o.INITIAL_SHARED_VERSION,
        o.OWNER_TYPE,
        o.OWNER_ADDRESS,
        o.CHECKPOINT,
        ROW_NUMBER() OVER (PARTITION BY o.OBJECT_ID ORDER BY o.VERSION DESC) as rn
    FROM analytics_db_v2.chaindata_mainnet.object o
    WHERE o.CHECKPOINT <= {TARGET_CHECKPOINT}
      AND (
          -- Pool wrapper
          o.OBJECT_ID = '{pool_config["pool_id"]}'
          -- PoolInner (owned by pool_inner_uid)
          OR o.OWNER_ADDRESS = '{pool_config["pool_inner_uid"]}'
          -- BigVector slices for bids
          OR o.OWNER_ADDRESS = '{pool_config["bids_bigvector"]}'
          -- BigVector slices for asks
          OR o.OWNER_ADDRESS = '{pool_config["asks_bigvector"]}'
      )
)
SELECT
    OBJECT_ID as object_id,
    OBJECT_TYPE as type,
    VERSION as version,
    OBJECT_JSON as object_json,
    INITIAL_SHARED_VERSION as initial_shared_version,
    OWNER_TYPE as owner_type,
    OWNER_ADDRESS as owner_address,
    CHECKPOINT as checkpoint
FROM latest_objects
WHERE rn = 1
"""


if __name__ == "__main__":
    print("DeepBook Pool State Export Script")
    print("=" * 50)

    for pool_key, pool_config in POOLS.items():
        print(f"\n{pool_config['name']} Pool:")
        print(f"  Pool ID: {pool_config['pool_id'][:20]}...")
        print(f"  Pool Inner: {pool_config['pool_inner_uid'][:20]}...")
        print(f"  Bids BigVector: {pool_config['bids_bigvector'][:20]}...")
        print(f"  Asks BigVector: {pool_config['asks_bigvector'][:20]}...")

    print(f"\nTarget Checkpoint: {TARGET_CHECKPOINT} (Feb 2, 2026)")
    print("\nTo export, run the SQL queries above in Snowflake")
    print("and save results as JSONL to ./data/{pool}_state_feb2_2026.jsonl")
