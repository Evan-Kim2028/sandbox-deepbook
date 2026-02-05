#!/usr/bin/env python3
"""
Run SQL queries to fetch all state objects and save to JSONL.
This script documents the exact queries needed.
"""

import json
import os
import sys

# All objects we need to fetch with their checkpoints
OBJECTS_TO_FETCH = [
    # Pool objects
    ("pool_wrapper", "0xe05dafb5133bcffb8d59f4e12465dc0e9faeaa05e3e342a08fe135800e3e4407", 241056077),
    ("pool_inner", "0x5c44ceb4c4e8ebb76813c729f8681a449ed1831129ac6e1cf966c7fcefe7dddb", 241056077),

    # Inner nodes
    ("asks_inner", "0xfe76d7120e2e1509b0ffa2113869955a493e75a52127e26a8952e8cd67415407", 241055195),
    ("bids_inner", "0xf33eb78a2218d83542c9db0192f698341dfa712f6ab9caf8712a1a973c162a8d", 241055195),

    # Asks leaf slices
    ("asks_382", "0x548774808ab28e348eb240383e0a4584b6a604f42dc0ba74fc25c83809a9d767", 241056077),
    ("asks_1412299", "0xfc08a74d515c4a8616270fc4dcea1b750eec6ea7b5c3eeeacb4c5b9e4defb7ba", 240216928),
    ("asks_1775705", "0xa48c26442894582fefa4e9a63d29c49553ff6f3c472172060143284a9b23a081", 240904030),
    ("asks_3157423", "0x92e5f96ab464ea1d94e8d29046807755eba794c09d884b479a471999bf14abb4", 240361090),
    ("asks_3627837", "0xfbc747771b6cb43370cf477ce0d2c8262454b2e1630d87f78a2f3a545feab57c", 240245437),
    ("asks_3712663", "0xf58a4cfc6c31eaf3715a4f9658b8b20f7a90a25938f597ba5eb251abe4806b0f", 241055195),
    ("asks_3727935", "0xb8802099f4330854d3838bdb765d64ef804366571cbf00ce4b7d400c9c7ea281", 240983604),
    ("asks_3766413", "0xf7b8a3d480039c016bca30170bafc2b2992e099be61cf0e838afc1fb6b45485a", 240303203),
    ("asks_3830237", "0x0818d0e929e137f444d04ee2525e2f2ee1736d42ffa1edad8b14d2119c72fe41", 241055195),

    # Bids leaf slices
    ("bids_102", "0x86b20501868938b9bc10bc695eddb80a0994a1298409286d26da80bf17347f46", 240553570),
    ("bids_619149", "0xe7aa909d63ce0a02d7072a0e50c8d70ef21408e9c74b0582b8ee3c413a19ad92", 240544857),
    ("bids_1371682", "0xb8c353b637d899962f8b66e14e19f58875c66a6ddc35349c11d78791e402b2d4", 241022507),
    ("bids_1504293", "0x8fdc5bd9931acb2164fad22fc967604b66b69542c75a8879ef3bf1fa96f3d73f", 241021163),
    ("bids_1532479", "0xa2704c9bc59146b6e232872270cb2c9bab738e20642204a05d86267f7cb8dead", 240953883),
    ("bids_1540409", "0x2be45f0fac371660454fb03155c3edd800490afb54e93a328ea57bc624d453ee", 241055195),
    ("bids_1541628", "0x59a9eb8a9e8a70d81df286f3e1b15e8565becc846f16a14b9e1a2e5fba319662", 241055195),
    ("bids_1541629", "0xfb86b890f51b5a5ed866450e6e8cc7211ebed70417193de3041a210b4d08d162", 241056075),
]

# Missing slices (not found in Snowflake)
MISSING_ASKS = ["3727881", "3727913"]


def generate_batch_query(objects):
    """Generate a batch SQL query to fetch multiple objects."""
    queries = []
    for name, obj_id, checkpoint in objects:
        query = f"""SELECT '{name}' as obj_name,
    o.OBJECT_ID as object_id, o.VERSION as version, o.OBJECT_JSON as object_json,
    o.STRUCT_TAG as object_type, o.OWNER_ADDRESS as owner_address, o.CHECKPOINT as checkpoint
FROM PIPELINE_V2_GROOT_DB.PIPELINE_V2_GROOT_SCHEMA.OBJECT_PARQUET2 o
WHERE o.OBJECT_ID = '{obj_id}'
AND o.CHECKPOINT = {checkpoint} AND o.OBJECT_JSON IS NOT NULL
QUALIFY ROW_NUMBER() OVER (PARTITION BY o.OBJECT_ID ORDER BY o.VERSION DESC) = 1"""
        queries.append(query)

    return "\n\nUNION ALL\n\n".join(queries)


def main():
    print("=== State Objects to Fetch ===")
    print(f"Total: {len(OBJECTS_TO_FETCH)} objects")
    print(f"Missing: {MISSING_ASKS}")

    print("\n=== Batch SQL Query ===\n")

    # Split into batches of 10 to avoid query size limits
    batch_size = 10
    for i in range(0, len(OBJECTS_TO_FETCH), batch_size):
        batch = OBJECTS_TO_FETCH[i:i+batch_size]
        print(f"\n-- Batch {i//batch_size + 1} ({len(batch)} objects)")
        print(generate_batch_query(batch))
        print(";")


if __name__ == "__main__":
    main()
