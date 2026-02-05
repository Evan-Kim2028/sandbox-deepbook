-- DeepBook V3 SUI/USDC Pool Complete State Export
-- This query exports all objects needed to fully reconstruct the orderbook
-- Run this against Snowflake to get the state snapshot

-- Configuration
SET pool_wrapper_id = '0xe05dafb5133bcffb8d59f4e12465dc0e9faeaa05e3e342a08fe135800e3e4407';
SET pool_inner_uid = '0x50997b5f1f6401674d3d881a61e09a71776ee19cd8b83114a0a21b3a82f130b5';
SET asks_bigvector = '0x5f8f0e3a2728a161e529ecacdfdface88b2fa669279aa699afd5d6b462c68466';
SET bids_bigvector = '0x090a8eae3204c76e36eebf3440cbde577e062953391760c37c363530fc1de246';
SET registry_id = '0xaf16199a2dff736e9f07a845f23c5da6df6f756eddb631aed9d24a93efc4549d';
SET min_checkpoint = 241600000;

-- Export all objects with latest version
CREATE OR REPLACE TEMPORARY TABLE deepbook_state_export AS
WITH latest_versions AS (
    SELECT
        OBJECT_ID,
        TYPE,
        VERSION,
        OBJECT_JSON,
        INITIAL_SHARED_VERSION,
        OWNER_TYPE,
        OWNER_ADDRESS,
        CHECKPOINT,
        ROW_NUMBER() OVER (PARTITION BY OBJECT_ID ORDER BY VERSION DESC) as rn
    FROM analytics_db_v2.chaindata_mainnet.object
    WHERE CHECKPOINT > $min_checkpoint
    AND (
        -- Pool wrapper
        OBJECT_ID = $pool_wrapper_id
        -- Dynamic fields owned by Pool wrapper (EWMA, etc.)
        OR OWNER_ADDRESS = $pool_wrapper_id
        -- PoolInner dynamic field (owned by inner UID)
        OR OWNER_ADDRESS = $pool_inner_uid
        -- Asks BigVector slices
        OR OWNER_ADDRESS = $asks_bigvector
        -- Bids BigVector slices
        OR OWNER_ADDRESS = $bids_bigvector
        -- Registry
        OR OBJECT_ID = $registry_id
    )
)
SELECT
    OBJECT_ID,
    TYPE,
    VERSION,
    OBJECT_JSON,
    INITIAL_SHARED_VERSION,
    OWNER_TYPE,
    OWNER_ADDRESS,
    CHECKPOINT
FROM latest_versions
WHERE rn = 1;

-- Summary of what was exported
SELECT
    CASE
        WHEN OBJECT_ID = $pool_wrapper_id THEN 'Pool Wrapper'
        WHEN OBJECT_ID = $registry_id THEN 'Registry'
        WHEN OWNER_ADDRESS = $pool_wrapper_id THEN 'Pool Dynamic Fields'
        WHEN OWNER_ADDRESS = $pool_inner_uid THEN 'PoolInner'
        WHEN OWNER_ADDRESS = $asks_bigvector THEN 'Asks Slices'
        WHEN OWNER_ADDRESS = $bids_bigvector THEN 'Bids Slices'
        ELSE 'Other'
    END as category,
    COUNT(*) as object_count,
    MAX(VERSION) as max_version,
    MAX(CHECKPOINT) as max_checkpoint
FROM deepbook_state_export
GROUP BY category
ORDER BY object_count DESC;

-- Export as JSONL for backend ingestion
-- Copy this output to a file: deepbook_state.jsonl
SELECT
    OBJECT_CONSTRUCT(
        'object_id', OBJECT_ID,
        'type', TYPE,
        'version', VERSION,
        'object_json', PARSE_JSON(OBJECT_JSON),
        'initial_shared_version', INITIAL_SHARED_VERSION,
        'owner_type', OWNER_TYPE,
        'owner_address', OWNER_ADDRESS,
        'checkpoint', CHECKPOINT
    )::STRING as state_object
FROM deepbook_state_export;

-- Simpler query for direct export (run this standalone)
-- Expected output: ~852 objects
WITH latest_versions AS (
    SELECT
        OBJECT_ID,
        TYPE,
        VERSION,
        OBJECT_JSON,
        INITIAL_SHARED_VERSION,
        OWNER_TYPE,
        OWNER_ADDRESS,
        CHECKPOINT,
        ROW_NUMBER() OVER (PARTITION BY OBJECT_ID ORDER BY VERSION DESC) as rn
    FROM analytics_db_v2.chaindata_mainnet.object
    WHERE CHECKPOINT > 241600000
    AND (
        OBJECT_ID = '0xe05dafb5133bcffb8d59f4e12465dc0e9faeaa05e3e342a08fe135800e3e4407'
        OR OWNER_ADDRESS = '0xe05dafb5133bcffb8d59f4e12465dc0e9faeaa05e3e342a08fe135800e3e4407'
        OR OWNER_ADDRESS = '0x50997b5f1f6401674d3d881a61e09a71776ee19cd8b83114a0a21b3a82f130b5'
        OR OWNER_ADDRESS = '0x5f8f0e3a2728a161e529ecacdfdface88b2fa669279aa699afd5d6b462c68466'
        OR OWNER_ADDRESS = '0x090a8eae3204c76e36eebf3440cbde577e062953391760c37c363530fc1de246'
        OR OBJECT_ID = '0xaf16199a2dff736e9f07a845f23c5da6df6f756eddb631aed9d24a93efc4549d'
    )
)
SELECT
    OBJECT_CONSTRUCT(
        'object_id', OBJECT_ID,
        'type', TYPE,
        'version', VERSION,
        'object_json', PARSE_JSON(OBJECT_JSON),
        'initial_shared_version', INITIAL_SHARED_VERSION,
        'owner_type', OWNER_TYPE,
        'owner_address', OWNER_ADDRESS,
        'checkpoint', CHECKPOINT
    )::STRING as state_object
FROM latest_versions
WHERE rn = 1;
