#!/usr/bin/env python3
"""
Script to document the queries needed to fetch complete SUI/USDC pool state.
Based on our investigation, we identified the following slices needed:

ASKS BigVector (parent: 0x5f8f0e3a2728a161e529ecacdfdface88b2fa669279aa699afd5d6b462c68466)
Inner node vals: [382, 3830237, 3712663, 3627837, 1775705, 1412299, 3727881, 3727913, 3727935, 3157423, 3766413]

Slices found and their last checkpoints:
- 382: found in 241M range (frequently updated)
- 1412299: object_id=0xfc08a74d..., last_cp=240216928
- 1775705: object_id=0xa48c2644..., last_cp=240904030
- 3157423: object_id=0x92e5f96a..., last_cp=240361090
- 3627837: object_id=0xfbc74777..., last_cp=240245437
- 3712663: object_id=0xf58a4cfc..., last_cp=241055195
- 3727935: object_id=0xb8802099..., last_cp=240983604
- 3766413: object_id=0xf7b8a3d4..., last_cp=240303203
- 3830237: object_id=0x0818d0e9..., last_cp=241055195

Missing (not found in 230M-241M range):
- 3727881
- 3727913

BIDS BigVector (parent: 0x090a8eae3204c76e36eebf3440cbde577e062953391760c37c363530fc1de246)
Inner node vals: [102, 619149, 1371682, 1504293, 1532479, 1540409, 1541628]

Strategy: Query for each slice at its specific checkpoint to avoid timeouts.
"""

# Object IDs and their last checkpoints
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

# Missing slices that weren't found
MISSING_ASKS = ["3727881", "3727913"]

def generate_slice_query(object_id: str, checkpoint: int) -> str:
    """Generate SQL query to fetch a slice at a specific checkpoint."""
    return f"""
SELECT o.OBJECT_ID as object_id, o.VERSION as version, o.OBJECT_JSON as object_json,
       o.STRUCT_TAG as object_type, o.OWNER_ADDRESS as owner_address, o.CHECKPOINT as checkpoint
FROM PIPELINE_V2_GROOT_DB.PIPELINE_V2_GROOT_SCHEMA.OBJECT_PARQUET2 o
WHERE o.OBJECT_ID = '{object_id}'
AND o.CHECKPOINT = {checkpoint}
AND o.OBJECT_JSON IS NOT NULL
LIMIT 1
"""

if __name__ == "__main__":
    print("=== Queries to fetch asks slices ===\n")
    for name, info in ASKS_SLICES.items():
        print(f"-- Slice {name}")
        print(generate_slice_query(info["object_id"], info["checkpoint"]))

    print("\n=== Missing slices ===")
    print(f"Still need to find: {MISSING_ASKS}")
    print("\nThese slices may have been created before our data range or may not exist.")
