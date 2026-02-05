#!/usr/bin/env python3
"""
Assemble complete SUI/USDC pool state from Snowflake data.
This script documents all the objects needed and their checkpoints.

Run this after collecting the data from Snowflake queries.
"""

import json
import os

# Output file
OUTPUT_FILE = "data/sui_usdc_state_complete.jsonl"

# Pool objects at checkpoint 241056077
POOL_OBJECTS = {
    "pool_wrapper": {
        "object_id": "0xe05dafb5133bcffb8d59f4e12465dc0e9faeaa05e3e342a08fe135800e3e4407",
        "checkpoint": 241056077,
    },
    "pool_inner": {
        "object_id": "0x5c44ceb4c4e8ebb76813c729f8681a449ed1831129ac6e1cf966c7fcefe7dddb",
        "checkpoint": 241056077,
    },
}

# BigVector inner nodes (Slice<u64>)
INNER_NODES = {
    "asks_inner": {
        "object_id": "0xfe76d7120e2e1509b0ffa2113869955a493e75a52127e26a8952e8cd67415407",
        "name": "856567",
        "checkpoint": 241055195,  # Latest found
    },
    "bids_inner": {
        "object_id": "0xf33eb78a2218d83542c9db0192f698341dfa712f6ab9caf8712a1a973c162a8d",
        "name": "298074",
        "checkpoint": 241055195,  # Latest found
    },
}

# Asks leaf slices (Slice<Order>)
# Parent: 0x5f8f0e3a2728a161e529ecacdfdface88b2fa669279aa699afd5d6b462c68466
ASKS_SLICES = {
    "382": {"object_id": "0x548774808ab28e348eb240383e0a4584b6a604f42dc0ba74fc25c83809a9d767", "checkpoint": 241056077},
    "1412299": {"object_id": "0xfc08a74d515c4a8616270fc4dcea1b750eec6ea7b5c3eeeacb4c5b9e4defb7ba", "checkpoint": 240216928},
    "1775705": {"object_id": "0xa48c26442894582fefa4e9a63d29c49553ff6f3c472172060143284a9b23a081", "checkpoint": 240904030},
    "3157423": {"object_id": "0x92e5f96ab464ea1d94e8d29046807755eba794c09d884b479a471999bf14abb4", "checkpoint": 240361090},
    "3627837": {"object_id": "0xfbc747771b6cb43370cf477ce0d2c8262454b2e1630d87f78a2f3a545feab57c", "checkpoint": 240245437},
    "3712663": {"object_id": "0xf58a4cfc6c31eaf3715a4f9658b8b20f7a90a25938f597ba5eb251abe4806b0f", "checkpoint": 241055195},
    "3727935": {"object_id": "0xb8802099f4330854d3838bdb765d64ef804366571cbf00ce4b7d400c9c7ea281", "checkpoint": 240983604},
    "3766413": {"object_id": "0xf7b8a3d480039c016bca30170bafc2b2992e099be61cf0e838afc1fb6b45485a", "checkpoint": 240303203},
    "3830237": {"object_id": "0x0818d0e929e137f444d04ee2525e2f2ee1736d42ffa1edad8b14d2119c72fe41", "checkpoint": 241055195},
}

# Missing asks slices (not found in Snowflake 230M-241M range)
MISSING_ASKS = ["3727881", "3727913"]

# Bids leaf slices (Slice<Order>)
# Parent: 0x090a8eae3204c76e36eebf3440cbde577e062953391760c37c363530fc1de246
BIDS_SLICES = {
    "102": {"object_id": "0x86b20501868938b9bc10bc695eddb80a0994a1298409286d26da80bf17347f46", "checkpoint": 240553570},
    "619149": {"object_id": "0xe7aa909d63ce0a02d7072a0e50c8d70ef21408e9c74b0582b8ee3c413a19ad92", "checkpoint": 240544857},
    "1371682": {"object_id": "0xb8c353b637d899962f8b66e14e19f58875c66a6ddc35349c11d78791e402b2d4", "checkpoint": 241022507},
    "1504293": {"object_id": "0x8fdc5bd9931acb2164fad22fc967604b66b69542c75a8879ef3bf1fa96f3d73f", "checkpoint": 241021163},
    "1532479": {"object_id": "0xa2704c9bc59146b6e232872270cb2c9bab738e20642204a05d86267f7cb8dead", "checkpoint": 240953883},
    "1540409": {"object_id": "0x2be45f0fac371660454fb03155c3edd800490afb54e93a328ea57bc624d453ee", "checkpoint": 241055195},
    "1541628": {"object_id": "0x59a9eb8a9e8a70d81df286f3e1b15e8565becc846f16a14b9e1a2e5fba319662", "checkpoint": 241055195},
    "1541629": {"object_id": "0xfb86b890f51b5a5ed866450e6e8cc7211ebed70417193de3041a210b4d08d162", "checkpoint": 241056075},
}


def generate_queries():
    """Generate SQL queries to fetch each object."""
    queries = []

    # Pool objects
    for name, info in POOL_OBJECTS.items():
        query = f"""-- {name}
SELECT o.OBJECT_ID as object_id, o.VERSION as version, o.OBJECT_JSON as object_json,
       o.STRUCT_TAG as object_type, o.OWNER_ADDRESS as owner_address, o.CHECKPOINT as checkpoint
FROM PIPELINE_V2_GROOT_DB.PIPELINE_V2_GROOT_SCHEMA.OBJECT_PARQUET2 o
WHERE o.OBJECT_ID = '{info["object_id"]}'
AND o.CHECKPOINT = {info["checkpoint"]}
AND o.OBJECT_JSON IS NOT NULL
ORDER BY o.VERSION DESC
LIMIT 1;
"""
        queries.append(query)

    # Inner nodes
    for name, info in INNER_NODES.items():
        query = f"""-- {name} (slice name {info["name"]})
SELECT o.OBJECT_ID as object_id, o.VERSION as version, o.OBJECT_JSON as object_json,
       o.STRUCT_TAG as object_type, o.OWNER_ADDRESS as owner_address, o.CHECKPOINT as checkpoint
FROM PIPELINE_V2_GROOT_DB.PIPELINE_V2_GROOT_SCHEMA.OBJECT_PARQUET2 o
WHERE o.OBJECT_ID = '{info["object_id"]}'
AND o.CHECKPOINT = {info["checkpoint"]}
AND o.OBJECT_JSON IS NOT NULL
ORDER BY o.VERSION DESC
LIMIT 1;
"""
        queries.append(query)

    # Asks slices
    for name, info in ASKS_SLICES.items():
        query = f"""-- asks slice {name}
SELECT o.OBJECT_ID as object_id, o.VERSION as version, o.OBJECT_JSON as object_json,
       o.STRUCT_TAG as object_type, o.OWNER_ADDRESS as owner_address, o.CHECKPOINT as checkpoint
FROM PIPELINE_V2_GROOT_DB.PIPELINE_V2_GROOT_SCHEMA.OBJECT_PARQUET2 o
WHERE o.OBJECT_ID = '{info["object_id"]}'
AND o.CHECKPOINT = {info["checkpoint"]}
AND o.OBJECT_JSON IS NOT NULL
ORDER BY o.VERSION DESC
LIMIT 1;
"""
        queries.append(query)

    # Bids slices
    for name, info in BIDS_SLICES.items():
        query = f"""-- bids slice {name}
SELECT o.OBJECT_ID as object_id, o.VERSION as version, o.OBJECT_JSON as object_json,
       o.STRUCT_TAG as object_type, o.OWNER_ADDRESS as owner_address, o.CHECKPOINT as checkpoint
FROM PIPELINE_V2_GROOT_DB.PIPELINE_V2_GROOT_SCHEMA.OBJECT_PARQUET2 o
WHERE o.OBJECT_ID = '{info["object_id"]}'
AND o.CHECKPOINT = {info["checkpoint"]}
AND o.OBJECT_JSON IS NOT NULL
ORDER BY o.VERSION DESC
LIMIT 1;
"""
        queries.append(query)

    return queries


def print_summary():
    """Print summary of objects to fetch."""
    print("=== SUI/USDC Pool State Objects ===\n")

    print(f"Pool Objects: {len(POOL_OBJECTS)}")
    for name, info in POOL_OBJECTS.items():
        print(f"  - {name}: {info['object_id'][:20]}... @ cp {info['checkpoint']}")

    print(f"\nInner Nodes: {len(INNER_NODES)}")
    for name, info in INNER_NODES.items():
        print(f"  - {name} (name={info['name']}): {info['object_id'][:20]}... @ cp {info['checkpoint']}")

    print(f"\nAsks Slices: {len(ASKS_SLICES)}")
    for name, info in ASKS_SLICES.items():
        print(f"  - {name}: {info['object_id'][:20]}... @ cp {info['checkpoint']}")

    print(f"\nMissing Asks Slices: {MISSING_ASKS}")

    print(f"\nBids Slices: {len(BIDS_SLICES)}")
    for name, info in BIDS_SLICES.items():
        print(f"  - {name}: {info['object_id'][:20]}... @ cp {info['checkpoint']}")

    total = len(POOL_OBJECTS) + len(INNER_NODES) + len(ASKS_SLICES) + len(BIDS_SLICES)
    print(f"\n=== Total Objects: {total} (excluding {len(MISSING_ASKS)} missing) ===")


if __name__ == "__main__":
    print_summary()
    print("\n\n=== SQL Queries to Execute ===\n")
    queries = generate_queries()
    for i, q in enumerate(queries, 1):
        print(f"-- Query {i}")
        print(q)
