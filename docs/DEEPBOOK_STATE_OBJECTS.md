# DeepBook V3 State Objects Reference

This document catalogs all objects required to hydrate DeepBook V3 SUI/USDC pool state for the sandbox frontend.

## Object Hierarchy

```
DeepBook Registry (Shared)
└── Inner Registry (Dynamic Field)

SUI/USDC Pool (Shared)
├── EWMA State (Dynamic Field)
├── Referral Rewards (Dynamic Field)
└── Inner Pool UID
    └── PoolInner (Dynamic Field)
        ├── Asks BigVector
        │   └── Order Slices (~1300 asks)
        └── Bids BigVector
            └── Order Slices (~330 bids)
```

## Essential Objects

### 1. Pool Wrapper
- **Object ID**: `0xe05dafb5133bcffb8d59f4e12465dc0e9faeaa05e3e342a08fe135800e3e4407`
- **Type**: `0x2c8d603bc51326b8c13cef9dd07031a408a48dddb541963357661df5d3204809::pool::Pool<0x2::sui::SUI, 0xdba34672e30cb065b1f93e3ab55318768fd6fef66c15942c9f7cb846e2f900e7::usdc::USDC>`
- **Owner**: Shared
- **Initial Shared Version**: 389750322
- **Points to**: Inner UID `0x50997b5f1f6401674d3d881a61e09a71776ee19cd8b83114a0a21b3a82f130b5`

### 2. PoolInner (Dynamic Field)
- **Object ID**: `0x5c44ceb4c4e8ebb76813c729f8681a449ed1831129ac6e1cf966c7fcefe7dddb`
- **Type**: `0x2::dynamic_field::Field<u64, pool::PoolInner<SUI, USDC>>`
- **Owner**: ObjectOwner (owned by inner UID)
- **Contains**:
  - `book.asks` BigVector: `0x5f8f0e3a2728a161e529ecacdfdface88b2fa669279aa699afd5d6b462c68466`
  - `book.bids` BigVector: `0x090a8eae3204c76e36eebf3440cbde577e062953391760c37c363530fc1de246`
  - `lot_size`: 100000000 (0.1 SUI)
  - `min_size`: 1000000000 (1 SUI)
  - `tick_size`: 100 (0.000001 USDC per tick)
  - Price data in `deep_price.quote_prices` array

### 3. EWMA State (Dynamic Field)
- **Object ID**: `0x80b1a10261cbe450d2c0948a06efc7066739526c95f0074afe9252eaa08785fb`
- **Type**: `0x2::dynamic_field::Field<vector<u8>, ewma::EWMAState>`
- **Owner**: Pool wrapper
- **Purpose**: Anti-manipulation price smoothing

### 4. Registry (Shared)
- **Object ID**: `0xaf16199a2dff736e9f07a845f23c5da6df6f756eddb631aed9d24a93efc4549d`
- **Type**: `0x2c8d603bc51326b8c13cef9dd07031a408a48dddb541963357661df5d3204809::registry::Registry`
- **Owner**: Shared
- **Points to**: Inner `0x4cc3af2ff1f4b5d41526a0a2cc24723b46e1236a216b24de022b1bf355bb01c2`

## BigVector Orderbook Structure

The orderbook uses BigVector for scalable storage:
- **Asks**: ~180 slice objects
- **Bids**: ~666 slice objects
- **Total**: ~846 orderbook slices

**Summary of all objects needed:**
| Category | Count |
|----------|-------|
| Pool Wrapper | 1 |
| PoolInner | 1 |
| Pool Dynamic Fields | 3 |
| Registry | 1 |
| Asks Slices | 180 |
| Bids Slices | 666 |
| **Total** | **~852** |

Each slice contains:
```json
{
  "name": "<slice_id>",
  "value": {
    "keys": ["<order_id_1>", "<order_id_2>", ...],
    "vals": [<Order objects>]
  }
}
```

## Packages Required

| Package | Address | Purpose |
|---------|---------|---------|
| Move Stdlib | `0x1` | Standard library |
| Sui Framework | `0x2` | Core framework (dynamic_field, object, etc.) |
| DeepBook Core | `0x2c8d603bc51326b8c13cef9dd07031a408a48dddb541963357661df5d3204809` | Pool, Order, BigVector |
| DeepBook V3 Upgrade | `0xcaf6ba059d539a97646d47f0b9ddf843e138d215e2a12ca1f4585d386f7aec3a` | V3 functions |
| USDC | `0xdba34672e30cb065b1f93e3ab55318768fd6fef66c15942c9f7cb846e2f900e7` | USDC coin type |

## Snowflake Query for State Export

```sql
-- Export all essential DeepBook objects at latest checkpoint
WITH essential_objects AS (
    -- Pool wrapper
    SELECT OBJECT_ID, TYPE, VERSION, OBJECT_JSON, INITIAL_SHARED_VERSION, OWNER_TYPE
    FROM analytics_db_v2.chaindata_mainnet.object
    WHERE OBJECT_ID = '0xe05dafb5133bcffb8d59f4e12465dc0e9faeaa05e3e342a08fe135800e3e4407'
    QUALIFY ROW_NUMBER() OVER (PARTITION BY OBJECT_ID ORDER BY VERSION DESC) = 1

    UNION ALL

    -- PoolInner
    SELECT OBJECT_ID, TYPE, VERSION, OBJECT_JSON, NULL as INITIAL_SHARED_VERSION, OWNER_TYPE
    FROM analytics_db_v2.chaindata_mainnet.object
    WHERE OBJECT_ID = '0x5c44ceb4c4e8ebb76813c729f8681a449ed1831129ac6e1cf966c7fcefe7dddb'
    QUALIFY ROW_NUMBER() OVER (PARTITION BY OBJECT_ID ORDER BY VERSION DESC) = 1

    UNION ALL

    -- EWMA State
    SELECT OBJECT_ID, TYPE, VERSION, OBJECT_JSON, NULL as INITIAL_SHARED_VERSION, OWNER_TYPE
    FROM analytics_db_v2.chaindata_mainnet.object
    WHERE OBJECT_ID = '0x80b1a10261cbe450d2c0948a06efc7066739526c95f0074afe9252eaa08785fb'
    QUALIFY ROW_NUMBER() OVER (PARTITION BY OBJECT_ID ORDER BY VERSION DESC) = 1

    UNION ALL

    -- Registry
    SELECT OBJECT_ID, TYPE, VERSION, OBJECT_JSON, INITIAL_SHARED_VERSION, OWNER_TYPE
    FROM analytics_db_v2.chaindata_mainnet.object
    WHERE OBJECT_ID = '0xaf16199a2dff736e9f07a845f23c5da6df6f756eddb631aed9d24a93efc4549d'
    QUALIFY ROW_NUMBER() OVER (PARTITION BY OBJECT_ID ORDER BY VERSION DESC) = 1
)
SELECT * FROM essential_objects;
```

## State Hydration Strategy

### Minimum Viable State (Fast Start)
1. Pool wrapper + PoolInner - Required for price info
2. Registry - Required for some pool operations
3. Packages (via gRPC) - DeepBook, Sui Framework, USDC

### Full State (Complete Orderbook)
Add all BigVector slices for complete order visibility.

### Hybrid Approach (Recommended)
1. Pre-load essential objects from Snowflake (fast)
2. Let sui-sandbox fetch BigVector slices on-demand via gRPC
3. Cache fetched slices for subsequent requests

## Notes

- Price conversion: `conversion_rate / 1e9` gives USDC/SUI price
- Example: `32383419689` → 3.238 USDC per SUI
- Lot size 100000000 = 0.1 SUI minimum order increment
- Tick size 100 = 0.0001 USDC price increment
