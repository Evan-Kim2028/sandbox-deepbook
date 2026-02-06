//! State Loader - Hydrates DeepBook state from Snowflake JSON exports
//!
//! This module converts Snowflake OBJECT_JSON data to BCS format using
//! the JsonToBcsConverter from sui-sandbox, then loads objects into
//! the SimulationEnvironment.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

/// Represents a single object exported from Snowflake
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportedObject {
    pub object_id: String,
    #[serde(alias = "type")]
    pub object_type: String,
    pub version: u64,
    pub object_json: serde_json::Value,
    #[serde(default)]
    pub initial_shared_version: Option<u64>,
    #[serde(default)]
    pub owner_type: Option<String>,
    #[serde(default)]
    pub owner_address: Option<String>,
    #[serde(default)]
    pub checkpoint: u64,
}

/// Pool identifier for the three supported pools
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PoolId {
    SuiUsdc,
    WalUsdc,
    DeepUsdc,
    DebugUsdc,
}

impl PoolId {
    pub fn as_str(&self) -> &'static str {
        match self {
            PoolId::SuiUsdc => "sui_usdc",
            PoolId::WalUsdc => "wal_usdc",
            PoolId::DeepUsdc => "deep_usdc",
            PoolId::DebugUsdc => "debug_usdc",
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            PoolId::SuiUsdc => "SUI/USDC",
            PoolId::WalUsdc => "WAL/USDC",
            PoolId::DeepUsdc => "DEEP/USDC",
            PoolId::DebugUsdc => "DBG/USDC",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "sui_usdc" | "sui-usdc" | "suiusdc" => Some(PoolId::SuiUsdc),
            "wal_usdc" | "wal-usdc" | "walusdc" => Some(PoolId::WalUsdc),
            "deep_usdc" | "deep-usdc" | "deepusdc" => Some(PoolId::DeepUsdc),
            "debug_usdc" | "debug-usdc" | "debugusdc" | "dbg_usdc" | "dbg-usdc" | "dbgusdc" => {
                Some(PoolId::DebugUsdc)
            }
            _ => None,
        }
    }

    pub fn all() -> &'static [PoolId] {
        &[PoolId::SuiUsdc, PoolId::WalUsdc, PoolId::DeepUsdc]
    }
}

/// DeepBook V3 object IDs and configuration for a single pool
#[derive(Debug, Clone)]
pub struct DeepBookConfig {
    /// Pool identifier
    pub pool_id: PoolId,
    /// Pool wrapper object ID
    pub pool_wrapper: String,
    /// Inner Pool UID (referenced by wrapper)
    pub pool_inner_uid: String,
    /// Asks BigVector ID
    pub asks_bigvector: String,
    /// Bids BigVector ID
    pub bids_bigvector: String,
    /// Base token decimals
    pub base_decimals: u8,
    /// Quote token decimals (USDC = 6)
    pub quote_decimals: u8,
    /// Registry ID (shared across all pools)
    pub registry: String,
    /// DeepBook package ID
    pub package: String,
}

impl DeepBookConfig {
    /// Create SUI/USDC pool configuration
    pub fn sui_usdc() -> Self {
        Self {
            pool_id: PoolId::SuiUsdc,
            pool_wrapper: "0xe05dafb5133bcffb8d59f4e12465dc0e9faeaa05e3e342a08fe135800e3e4407"
                .to_string(),
            pool_inner_uid: "0x50997b5f1f6401674d3d881a61e09a71776ee19cd8b83114a0a21b3a82f130b5"
                .to_string(),
            asks_bigvector: "0x5f8f0e3a2728a161e529ecacdfdface88b2fa669279aa699afd5d6b462c68466"
                .to_string(),
            bids_bigvector: "0x090a8eae3204c76e36eebf3440cbde577e062953391760c37c363530fc1de246"
                .to_string(),
            base_decimals: 9,  // SUI has 9 decimals
            quote_decimals: 6, // USDC has 6 decimals
            registry: "0xaf16199a2dff736e9f07a845f23c5da6df6f756eddb631aed9d24a93efc4549d"
                .to_string(),
            package: "0x2c8d603bc51326b8c13cef9dd07031a408a48dddb541963357661df5d3204809"
                .to_string(),
        }
    }

    /// Create WAL/USDC pool configuration
    pub fn wal_usdc() -> Self {
        Self {
            pool_id: PoolId::WalUsdc,
            pool_wrapper: "0x56a1c985c1f1123181d6b881714793689321ba24301b3585eec427436eb1c76d"
                .to_string(),
            pool_inner_uid: "0xe28eca4e6470c7a326f58eadb0482665b5f0831be0c1a0f8f33a0a998729f0d3"
                .to_string(),
            asks_bigvector: "0x1bf5e16fcfb6c4d293c550bc1333ec7a6ed8323a929bb2db477f63ff0e9b6a4c"
                .to_string(),
            bids_bigvector: "0x82ee32196ab12750268815e005fae4c4db23a4272e52610c0c25a8288f05515a"
                .to_string(),
            base_decimals: 9,  // WAL has 9 decimals
            quote_decimals: 6, // USDC has 6 decimals
            registry: "0xaf16199a2dff736e9f07a845f23c5da6df6f756eddb631aed9d24a93efc4549d"
                .to_string(),
            package: "0x2c8d603bc51326b8c13cef9dd07031a408a48dddb541963357661df5d3204809"
                .to_string(),
        }
    }

    /// Create DEEP/USDC pool configuration
    pub fn deep_usdc() -> Self {
        Self {
            pool_id: PoolId::DeepUsdc,
            pool_wrapper: "0xf948981b806057580f91622417534f491da5f61aeaf33d0ed8e69fd5691c95ce"
                .to_string(),
            pool_inner_uid: "0xac73b6fd7dfca972f1f583c3b59daa110cb44c9a3419cf697533f87e9e7bb7f4"
                .to_string(),
            asks_bigvector: "0x0f9d6fc9de7a0ee0dd98f7326619cd5ff74cc0bc6485cce80014f766e437c4ae"
                .to_string(),
            bids_bigvector: "0xd1fcd1d0a554150fa097508eabcd76f6dbb0d2ce4fdfeffb2f6a4469ac81fd42"
                .to_string(),
            base_decimals: 6,  // DEEP has 6 decimals
            quote_decimals: 6, // USDC has 6 decimals
            registry: "0xaf16199a2dff736e9f07a845f23c5da6df6f756eddb631aed9d24a93efc4549d"
                .to_string(),
            package: "0x2c8d603bc51326b8c13cef9dd07031a408a48dddb541963357661df5d3204809"
                .to_string(),
        }
    }

    /// Get config for a specific pool
    pub fn for_pool(pool_id: PoolId) -> Self {
        match pool_id {
            PoolId::SuiUsdc => Self::sui_usdc(),
            PoolId::WalUsdc => Self::wal_usdc(),
            PoolId::DeepUsdc => Self::deep_usdc(),
            PoolId::DebugUsdc => Self::sui_usdc(),
        }
    }
}

impl Default for DeepBookConfig {
    fn default() -> Self {
        Self::sui_usdc()
    }
}

/// Manages loading and caching of DeepBook state
pub struct StateLoader {
    config: DeepBookConfig,
    /// Cached objects indexed by object_id
    objects: HashMap<String, ExportedObject>,
    /// Whether state has been loaded
    loaded: bool,
}

impl StateLoader {
    /// Create a new StateLoader with default configuration
    pub fn new() -> Self {
        Self {
            config: DeepBookConfig::default(),
            objects: HashMap::new(),
            loaded: false,
        }
    }

    /// Create with custom configuration
    pub fn with_config(config: DeepBookConfig) -> Self {
        Self {
            config,
            objects: HashMap::new(),
            loaded: false,
        }
    }

    /// Load state from a JSON/JSONL file exported from Snowflake
    /// Auto-detects format based on file extension
    pub fn load_from_file(&mut self, path: &Path) -> Result<usize, Box<dyn std::error::Error>> {
        let content = std::fs::read_to_string(path)?;

        // Auto-detect format based on extension
        let is_jsonl = path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("jsonl"));

        if is_jsonl {
            self.load_from_jsonl(&content)
        } else {
            self.load_from_json(&content)
        }
    }

    /// Load state from JSON string (array of ExportedObject)
    pub fn load_from_json(&mut self, json: &str) -> Result<usize, Box<dyn std::error::Error>> {
        let objects: Vec<ExportedObject> = serde_json::from_str(json)?;

        let count = objects.len();
        for obj in objects {
            self.objects.insert(obj.object_id.clone(), obj);
        }

        self.loaded = true;
        Ok(count)
    }

    /// Load state from JSONL format (one object per line)
    ///
    /// When multiple versions of the same object exist, keeps only the one
    /// with the highest version number (most recent state).
    pub fn load_from_jsonl(&mut self, jsonl: &str) -> Result<usize, Box<dyn std::error::Error>> {
        let mut count = 0;
        for line in jsonl.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let obj: ExportedObject = serde_json::from_str(line)?;

            // Only insert if this is a newer version than what we have
            let should_insert = match self.objects.get(&obj.object_id) {
                Some(existing) => obj.version > existing.version,
                None => true,
            };

            if should_insert {
                self.objects.insert(obj.object_id.clone(), obj);
            }
            count += 1;
        }

        self.loaded = true;
        Ok(count)
    }

    /// Check if state has been loaded
    pub fn is_loaded(&self) -> bool {
        self.loaded
    }

    /// Get the number of loaded objects
    pub fn object_count(&self) -> usize {
        self.objects.len()
    }

    /// Get an object by ID
    pub fn get_object(&self, object_id: &str) -> Option<&ExportedObject> {
        self.objects.get(object_id)
    }

    /// Get the Pool wrapper object
    pub fn get_pool(&self) -> Option<&ExportedObject> {
        self.objects.get(&self.config.pool_wrapper)
    }

    /// Get the PoolInner object
    pub fn get_pool_inner(&self) -> Option<&ExportedObject> {
        self.objects.values().find(|obj| {
            obj.owner_address.as_ref() == Some(&self.config.pool_inner_uid)
                && obj.object_type.contains("PoolInner")
        })
    }

    /// Get all orderbook slice objects
    pub fn get_orderbook_slices(&self) -> Vec<&ExportedObject> {
        self.objects
            .values()
            .filter(|obj| {
                obj.owner_address.as_ref().is_some_and(|addr| {
                    addr == &self.config.asks_bigvector || addr == &self.config.bids_bigvector
                })
            })
            .collect()
    }

    /// Get asks slices
    pub fn get_asks_slices(&self) -> Vec<&ExportedObject> {
        self.objects
            .values()
            .filter(|obj| obj.owner_address.as_ref() == Some(&self.config.asks_bigvector))
            .collect()
    }

    /// Get bids slices
    pub fn get_bids_slices(&self) -> Vec<&ExportedObject> {
        self.objects
            .values()
            .filter(|obj| obj.owner_address.as_ref() == Some(&self.config.bids_bigvector))
            .collect()
    }

    /// Get statistics about loaded state
    pub fn stats(&self) -> StateStats {
        let asks_count = self.get_asks_slices().len();
        let bids_count = self.get_bids_slices().len();

        let max_checkpoint = self
            .objects
            .values()
            .map(|o| o.checkpoint)
            .max()
            .unwrap_or(0);
        let max_version = self.objects.values().map(|o| o.version).max().unwrap_or(0);

        StateStats {
            total_objects: self.objects.len(),
            asks_slices: asks_count,
            bids_slices: bids_count,
            max_checkpoint,
            max_version,
        }
    }

    /// Get the configuration
    pub fn config(&self) -> &DeepBookConfig {
        &self.config
    }

    /// Extract all objects for conversion
    pub fn all_objects(&self) -> impl Iterator<Item = &ExportedObject> {
        self.objects.values()
    }

    /// Get objects by owner address
    pub fn get_by_owner(&self, owner: &str) -> Vec<&ExportedObject> {
        self.objects
            .values()
            .filter(|obj| obj.owner_address.as_ref() == Some(&owner.to_string()))
            .collect()
    }
}

impl Default for StateLoader {
    fn default() -> Self {
        Self::new()
    }
}

/// Statistics about loaded state
#[derive(Debug, Clone, Serialize)]
pub struct StateStats {
    pub total_objects: usize,
    pub asks_slices: usize,
    pub bids_slices: usize,
    pub max_checkpoint: u64,
    pub max_version: u64,
}

/// Registry managing multiple pool state loaders
pub struct PoolRegistry {
    pools: HashMap<PoolId, StateLoader>,
}

impl PoolRegistry {
    /// Create a new empty registry
    pub fn new() -> Self {
        Self {
            pools: HashMap::new(),
        }
    }

    /// Load a pool state from a file
    pub fn load_pool_from_file(
        &mut self,
        pool_id: PoolId,
        path: &Path,
    ) -> Result<usize, Box<dyn std::error::Error>> {
        let config = DeepBookConfig::for_pool(pool_id);
        let mut loader = StateLoader::with_config(config);
        let count = loader.load_from_file(path)?;
        self.pools.insert(pool_id, loader);
        Ok(count)
    }

    /// Get a loader for a specific pool
    pub fn get(&self, pool_id: PoolId) -> Option<&StateLoader> {
        self.pools.get(&pool_id)
    }

    /// Get all loaded pool IDs
    pub fn loaded_pools(&self) -> Vec<PoolId> {
        self.pools.keys().copied().collect()
    }

    /// Check if a pool is loaded
    pub fn is_loaded(&self, pool_id: PoolId) -> bool {
        self.pools.get(&pool_id).is_some_and(|l| l.is_loaded())
    }

    /// Get summary statistics for all loaded pools
    pub fn summary(&self) -> RegistrySummary {
        let pools: Vec<PoolSummary> = self
            .pools
            .iter()
            .map(|(id, loader)| {
                let stats = loader.stats();
                PoolSummary {
                    pool_id: *id,
                    pool_name: id.display_name().to_string(),
                    total_objects: stats.total_objects,
                    asks_slices: stats.asks_slices,
                    bids_slices: stats.bids_slices,
                    checkpoint: stats.max_checkpoint,
                }
            })
            .collect();

        RegistrySummary {
            total_pools: pools.len(),
            pools,
        }
    }
}

impl Default for PoolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Summary of all loaded pools
#[derive(Debug, Clone, Serialize)]
pub struct RegistrySummary {
    pub total_pools: usize,
    pub pools: Vec<PoolSummary>,
}

/// Summary for a single pool
#[derive(Debug, Clone, Serialize)]
pub struct PoolSummary {
    pub pool_id: PoolId,
    pub pool_name: String,
    pub total_objects: usize,
    pub asks_slices: usize,
    pub bids_slices: usize,
    pub checkpoint: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_state_loader_default() {
        let loader = StateLoader::new();
        assert!(!loader.is_loaded());
        assert_eq!(loader.object_count(), 0);
    }

    #[test]
    fn test_load_empty_json() {
        let mut loader = StateLoader::new();
        let result = loader.load_from_json("[]");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 0);
        assert!(loader.is_loaded());
    }

    #[test]
    fn test_load_single_object() {
        let mut loader = StateLoader::new();
        let json = r#"[{
            "object_id": "0x123",
            "type": "0x2::coin::Coin<0x2::sui::SUI>",
            "version": 100,
            "object_json": {"value": "1000"},
            "initial_shared_version": null,
            "owner_type": "AddressOwner",
            "owner_address": "0xabc",
            "checkpoint": 12345
        }]"#;

        let result = loader.load_from_json(json);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 1);
        assert!(loader.get_object("0x123").is_some());
    }

    #[test]
    fn test_default_config() {
        let config = DeepBookConfig::default();
        assert!(config.pool_wrapper.starts_with("0x"));
        assert!(config.package.starts_with("0x"));
    }
}
