//! DeepBook Orderbook Builder using sui-sandbox
//!
//! This module uses the sui-sandbox SimulationEnvironment to call DeepBook's
//! `iter_orders` view function, which properly decodes order data from the
//! Move VM rather than manually parsing BCS.

use anyhow::{anyhow, Result};
use move_core_types::account_address::AccountAddress;
use move_core_types::identifier::Identifier;
use move_core_types::language_storage::{StructTag, TypeTag};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::str::FromStr;

use sui_sandbox_core::fetcher::GrpcFetcher;
use sui_sandbox_core::ptb::{Argument, Command, InputValue, ObjectInput};
use sui_sandbox_core::simulation::state::FetcherConfig;
use sui_sandbox_core::simulation::SimulationEnvironment;

use super::snowflake_bcs::JsonToBcsConverter;
use super::state_loader::{ExportedObject, PoolId, StateLoader};

// Note: gRPC is only used for package loading, not for fetching missing slices
// All pool state should come from Snowflake data

// DeepBook V3 Package
const DEEPBOOK_PACKAGE: &str = "0x2c8d603bc51326b8c13cef9dd07031a408a48dddb541963357661df5d3204809";

// Type tags for assets
const SUI_TYPE: &str = "0x2::sui::SUI";
const USDC_TYPE: &str =
    "0xdba34672e30cb065b1f93e3ab55318768fd6fef66c15942c9f7cb846e2f900e7::usdc::USDC";
const WAL_TYPE: &str =
    "0x356a26eb9e012a68958082340d4c4116e7f55615cf27affcff209cf0ae544f59::wal::WAL";
const DEEP_TYPE: &str =
    "0xdeeb7a4662eec9f2f3def03fb937a663dddaa2e215b8078a284d026b7946c270::deep::DEEP";

/// Order from DeepBook (decoded by Move VM)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecodedOrder {
    pub order_id: u128,
    pub price: u64,           // Decoded from order_id by the contract
    pub quantity: u64,        // Original quantity in base units
    pub filled_quantity: u64, // Already filled
    pub is_bid: bool,
    pub expire_timestamp: u64,
}

impl DecodedOrder {
    /// Remaining quantity (unfilled)
    pub fn remaining_quantity(&self) -> u64 {
        self.quantity.saturating_sub(self.filled_quantity)
    }

    /// Price in human-readable format (assumes 6 decimal quote)
    pub fn price_usd(&self, quote_decimals: u8) -> f64 {
        self.price as f64 / 10f64.powi(quote_decimals as i32)
    }

    /// Quantity in human-readable format
    pub fn quantity_human(&self, base_decimals: u8) -> f64 {
        self.remaining_quantity() as f64 / 10f64.powi(base_decimals as i32)
    }
}

/// Price level aggregated from multiple orders
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PriceLevel {
    pub price: u64,
    pub total_quantity: u64,
    pub order_count: usize,
}

/// Complete orderbook built from sui-sandbox execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxOrderbook {
    pub pool_id: PoolId,
    pub bids: Vec<PriceLevel>, // Sorted descending by price
    pub asks: Vec<PriceLevel>, // Sorted ascending by price
    pub checkpoint: u64,
    pub base_decimals: u8,
    pub quote_decimals: u8,
}

impl SandboxOrderbook {
    /// Price normalization factor to convert from DeepBook's internal representation
    /// DeepBook V3 normalizes all prices as if base tokens have 9 decimals
    /// So for tokens with fewer decimals, we need to divide by 10^(9 - base_decimals)
    fn price_divisor(&self) -> f64 {
        self.price_divisor_value()
    }

    /// Public accessor for the price divisor
    pub fn price_divisor_value(&self) -> f64 {
        // USDC quote decimals (10^6) * normalization factor (10^(9 - base_decimals))
        let normalization = 10f64.powi(9 - self.base_decimals as i32);
        1_000_000.0 * normalization
    }

    pub fn mid_price(&self) -> Option<f64> {
        let best_bid = self.bids.first().map(|l| l.price)?;
        let best_ask = self.asks.first().map(|l| l.price)?;
        Some((best_bid + best_ask) as f64 / 2.0 / self.price_divisor())
    }

    pub fn best_bid(&self) -> Option<f64> {
        self.bids
            .first()
            .map(|l| l.price as f64 / self.price_divisor())
    }

    pub fn best_ask(&self) -> Option<f64> {
        self.asks
            .first()
            .map(|l| l.price as f64 / self.price_divisor())
    }

    pub fn spread_bps(&self) -> Option<u64> {
        let best_bid = self.bids.first().map(|l| l.price)?;
        let best_ask = self.asks.first().map(|l| l.price)?;
        if best_bid == 0 || best_ask == 0 {
            return None;
        }
        let mid = (best_bid + best_ask) / 2;
        if mid == 0 {
            return None;
        }
        // spread in basis points = (ask - bid) / mid * 10000
        let spread = best_ask.abs_diff(best_bid);
        Some(spread * 10000 / mid)
    }
}

/// Builder that uses sui-sandbox to construct orderbooks
pub struct OrderbookBuilder {
    env: SimulationEnvironment,
    packages_loaded: bool,
    /// Cached pool objects (object_id -> (bcs_bytes, type_tag, version))
    pool_cache: HashMap<String, (Vec<u8>, TypeTag, u64)>,
    /// JSON to BCS converter with bytecode layouts loaded
    bcs_converter: JsonToBcsConverter,
    /// Track missing slice names for debugging
    missing_slices: Vec<(String, u64)>, // (parent_uid, slice_name)
}

impl OrderbookBuilder {
    /// Create a new builder
    pub fn new() -> Result<Self> {
        let env = SimulationEnvironment::new()?;
        Ok(Self {
            env,
            packages_loaded: false,
            pool_cache: HashMap::new(),
            bcs_converter: JsonToBcsConverter::new(),
            missing_slices: Vec::new(),
        })
    }

    /// Load packages from gRPC (Move Stdlib, Sui Framework, DeepBook)
    /// Also configures the environment to auto-fetch any missing package dependencies.
    pub async fn load_packages_from_grpc(&mut self) -> Result<()> {
        let rt = tokio::runtime::Handle::current();

        let grpc = rt
            .spawn(async { sui_transport::grpc::GrpcClient::mainnet().await })
            .await??;

        // Configure the environment with a GrpcFetcher for auto-fetching missing packages
        let fetcher = GrpcFetcher::mainnet();
        let config = FetcherConfig::mainnet();
        self.env.set_fetcher(Box::new(fetcher));
        self.env.set_fetcher_config(config);
        tracing::info!("Configured auto-fetch for missing packages");

        // Core packages to load explicitly (for BCS converter layouts and linking)
        let packages_to_fetch = [
            ("0x1", "Move Stdlib"),
            ("0x2", "Sui Framework"),
            (DEEPBOOK_PACKAGE, "DeepBook V3"),
            // Token packages needed for Pool<BaseAsset, QuoteAsset> type resolution
            (USDC_TYPE.split("::").next().unwrap(), "USDC"),
            (WAL_TYPE.split("::").next().unwrap(), "WAL"),
            (DEEP_TYPE.split("::").next().unwrap(), "DEEP"),
            // DeepBook package dependencies (discovered through linker errors)
            (
                "0xe0917b74a5912e4ad186ac634e29c922ab83903f71af7500969f9411706f9b9a",
                "Upgrade Service",
            ),
            (
                "0xecf47609d7da919ea98e7fd04f6e0648a0a79b337aaad373fa37aac8febf19c8",
                "Treasury",
            ),
        ];

        for (pkg_id, name) in &packages_to_fetch {
            if let Ok(Some(obj)) = grpc.get_object(pkg_id).await {
                if let Some(modules) = obj.package_modules {
                    // Collect bytecode for the BCS converter
                    // modules is Vec<(String, Vec<u8>)> where each tuple is (module_name, bytecode)
                    let bytecode_list: Vec<Vec<u8>> =
                        modules.iter().map(|(_, bytes)| bytes.clone()).collect();

                    // Add to BCS converter for layout resolution
                    if let Err(e) = self.bcs_converter.add_modules_from_bytes(&bytecode_list) {
                        tracing::warn!("Failed to add {} to BCS converter: {}", name, e);
                    }

                    // Deploy package to simulation environment
                    self.env.deploy_package_at_address(pkg_id, modules)?;
                    tracing::info!("Loaded {} ({} modules)", name, pkg_id);
                }
            }
        }

        self.packages_loaded = true;
        Ok(())
    }

    /// Load packages from bundled bytecode (faster, no network)
    ///
    /// Note: Sui Framework (0x1, 0x2) is automatically loaded by SimulationEnvironment::new().
    /// DeepBook package must still be loaded from gRPC.
    pub fn load_packages_from_bundled(&mut self) -> Result<()> {
        // Sui framework is already loaded by SimulationEnvironment::new()
        // For DeepBook, we need to fetch from network or have pre-cached
        tracing::warn!(
            "DeepBook package requires gRPC loading - use load_packages_from_grpc() instead"
        );
        self.packages_loaded = true;
        Ok(())
    }

    /// Load pool state from a StateLoader
    ///
    /// Note: This converts JSON object data to BCS using bytecode layouts.
    /// Packages must be loaded first via load_packages_from_grpc().
    /// Dynamic field objects are registered using set_dynamic_field for proper Move VM resolution.
    pub fn load_pool_state(&mut self, loader: &StateLoader, pool_id: PoolId) -> Result<()> {
        if !loader.is_loaded() {
            return Err(anyhow!("StateLoader has no data loaded"));
        }

        if !self.packages_loaded {
            return Err(anyhow!(
                "Packages must be loaded before pool state. Call load_packages_from_grpc() first"
            ));
        }

        // Get the pool wrapper object
        let config = loader.config();
        let pool_wrapper_id = &config.pool_wrapper;

        // Load all objects from the state loader
        for obj in loader.all_objects() {
            // Check if this is a dynamic field (has an owner_address that's another object)
            if let Some(owner_addr) = &obj.owner_address {
                if obj.object_type.contains("dynamic_field::Field") {
                    // This is a dynamic field - register it properly
                    self.load_dynamic_field(obj, owner_addr)?;
                    continue;
                }
            }

            // Regular object - load normally
            self.load_object(obj)?;

            // Cache the pool wrapper for later use
            if obj.object_id == *pool_wrapper_id {
                // Convert object_json to BCS bytes using the converter
                let bcs_bytes = match self
                    .bcs_converter
                    .convert(&obj.object_type, &obj.object_json)
                {
                    Ok(bytes) => bytes,
                    Err(e) => {
                        tracing::warn!(
                            "BCS conversion failed for pool {}, using JSON fallback: {}",
                            obj.object_id,
                            e
                        );
                        serde_json::to_vec(&obj.object_json)?
                    }
                };

                // Build the Pool type tag
                let (base_type, quote_type) = match pool_id {
                    PoolId::SuiUsdc => (SUI_TYPE, USDC_TYPE),
                    PoolId::WalUsdc => (WAL_TYPE, USDC_TYPE),
                    PoolId::DeepUsdc => (DEEP_TYPE, USDC_TYPE),
                };

                let pool_type = build_pool_type_tag(base_type, quote_type)?;
                self.pool_cache
                    .insert(pool_wrapper_id.clone(), (bcs_bytes, pool_type, obj.version));
            }
        }

        Ok(())
    }

    /// Analyze and report missing BigVector slices
    ///
    /// This checks which slices are referenced by inner nodes but not present
    /// in the Snowflake export. Missing slices should be fetched from Snowflake
    /// at earlier checkpoints.
    ///
    /// Returns the list of missing slices as (parent_uid, slice_name) tuples.
    pub fn analyze_missing_slices(&mut self, loader: &StateLoader) -> Vec<(String, u64)> {
        // Find inner nodes and extract their vals (child slice names)
        let mut inner_node_vals: HashMap<String, Vec<u64>> = HashMap::new();
        let mut loaded_slice_names: HashMap<String, std::collections::HashSet<u64>> =
            HashMap::new();

        for obj in loader.all_objects() {
            if obj.object_type.contains("big_vector::Slice<u64>") {
                // This is an inner node - extract vals
                if let Some(owner) = &obj.owner_address {
                    if let Some(value) = obj.object_json.get("value") {
                        if let Some(vals) = value.get("vals") {
                            if let Some(arr) = vals.as_array() {
                                let slice_names: Vec<u64> = arr
                                    .iter()
                                    .filter_map(|v| v.as_str().and_then(|s| s.parse().ok()))
                                    .collect();
                                inner_node_vals.insert(owner.clone(), slice_names);
                            }
                        }
                    }
                }
            } else if obj.object_type.contains("big_vector::Slice<")
                && obj.object_type.contains("Order")
            {
                // This is a leaf node - track its name
                if let Some(owner) = &obj.owner_address {
                    if let Some(name) = obj.object_json.get("name") {
                        if let Some(name_str) = name.as_str() {
                            if let Ok(name_u64) = name_str.parse::<u64>() {
                                loaded_slice_names
                                    .entry(owner.clone())
                                    .or_default()
                                    .insert(name_u64);
                            }
                        }
                    }
                }
            }
        }

        // Find missing slices
        let mut missing: Vec<(String, u64)> = Vec::new();
        for (parent, vals) in &inner_node_vals {
            let loaded = loaded_slice_names.get(parent).cloned().unwrap_or_default();
            for &val in vals {
                if !loaded.contains(&val) {
                    missing.push((parent.clone(), val));
                }
            }
        }

        if missing.is_empty() {
            tracing::info!("All referenced slices are present in Snowflake data");
        } else {
            tracing::warn!(
                "Found {} missing slices - these need to be fetched from Snowflake at earlier checkpoints",
                missing.len()
            );
            for (parent, name) in &missing {
                tracing::warn!("Missing slice: parent={}, name={}", parent, name);
            }
        }

        // Store for later reference
        self.missing_slices = missing.clone();
        missing
    }

    /// Get the list of missing slices identified during analysis
    pub fn get_missing_slices(&self) -> &[(String, u64)] {
        &self.missing_slices
    }

    /// Load a dynamic field object into the simulation environment
    ///
    /// Dynamic fields need to be registered with set_dynamic_field for the Move VM's
    /// dynamic_field module to find them during execution.
    ///
    /// IMPORTANT: The Move VM's `borrow_child_object<Field<K, V>>` expects the FULL
    /// `Field<K, V>` type, not just the VALUE type. And the BCS bytes should be the
    /// full Field struct (id, name, value), not just the value.
    fn load_dynamic_field(&mut self, obj: &ExportedObject, parent_addr: &str) -> Result<()> {
        // Parse the parent and child addresses
        let parent_id = AccountAddress::from_hex_literal(parent_addr)?;
        let child_id = AccountAddress::from_hex_literal(&obj.object_id)?;

        // Fix type for BigVector slices - Snowflake export has incorrect types
        // The export has: Field<u64, vector<Order>>
        // But the actual type depends on whether it's a leaf or inner node:
        //   - Leaf nodes: Field<u64, Slice<Order>> (vals contains Order objects)
        //   - Inner nodes: Field<u64, Slice<u64>> (vals contains child slice IDs)
        let corrected_type = self.correct_bigvector_slice_type(&obj.object_type, &obj.object_json);
        if corrected_type != obj.object_type {
            eprintln!(
                "DEBUG type correction: {} -> {}",
                obj.object_type, corrected_type
            );
        }

        // Parse the FULL Field<K, V> type - this is what borrow_child_object expects
        let full_type_tag = SimulationEnvironment::parse_type_string(&corrected_type)
            .ok_or_else(|| anyhow!("Failed to parse field type: {}", corrected_type))?;

        // Convert the FULL Field object to BCS bytes (id + name + value)
        // For BigVector slices, use the corrected type for BCS conversion
        let bcs_bytes = match self
            .bcs_converter
            .convert(&corrected_type, &obj.object_json)
        {
            Ok(bytes) => bytes,
            Err(e) => {
                // Print error for Slice types - this is critical
                if corrected_type.contains("Slice") {
                    eprintln!(
                        "ERROR: BCS conversion failed for Slice {}: {}",
                        obj.object_id, e
                    );
                }
                tracing::debug!(
                    "BCS conversion for dynamic field {} (type: {}): {}",
                    obj.object_id,
                    corrected_type,
                    e
                );
                // Use JSON serialization as fallback - THIS WILL FAIL AT RUNTIME
                serde_json::to_vec(&obj.object_json)?
            }
        };

        // Register the dynamic field with the simulation environment
        // Use the FULL Field<K, V> type, which is what borrow_child_object<Field<K,V>> expects
        // Debug: print first 100 bytes of BCS
        if corrected_type.contains("Slice") {
            eprintln!(
                "DEBUG BCS for Slice: first 100 bytes = {:02x?}",
                &bcs_bytes[..std::cmp::min(100, bcs_bytes.len())]
            );
        }
        self.env.set_dynamic_field(
            parent_id,
            child_id,
            full_type_tag.clone(),
            bcs_bytes.clone(),
        );

        tracing::info!(
            "Registered dynamic field: parent={}, child={}, type={}, bcs_len={}",
            parent_addr,
            obj.object_id,
            full_type_tag,
            bcs_bytes.len()
        );

        // Note: Only use set_dynamic_field for dynamic fields.
        // Do NOT also call load_object_from_data as this may conflict.

        Ok(())
    }

    /// Correct the type string for BigVector slices
    ///
    /// Snowflake exports BigVector slices with incorrect types:
    ///   Field<u64, vector<Element>>
    /// But the actual Move type depends on whether it's a leaf or inner node:
    ///   - Leaf nodes: Field<u64, Slice<Element>> (vals contains Element objects)
    ///   - Inner nodes: Field<u64, Slice<u64>> (vals contains child slice IDs)
    fn correct_bigvector_slice_type(&self, type_str: &str, json: &serde_json::Value) -> String {
        // Check if this looks like a BigVector slice (Field<u64, vector<...>>)
        // where the element type is from DeepBook (contains the DeepBook package address)
        if type_str.contains("::dynamic_field::Field<u64, vector<")
            && type_str.contains(DEEPBOOK_PACKAGE)
        {
            // Detect if this is an inner node or leaf node by checking vals content
            // Inner nodes have vals as array of strings (u64 IDs)
            // Leaf nodes have vals as array of objects (Order structs)
            let is_inner_node = json
                .get("value")
                .and_then(|v| v.get("vals"))
                .and_then(|vals| vals.as_array())
                .map(|arr| {
                    // If vals is empty or first element is a string, it's an inner node
                    arr.first().map(|v| v.is_string()).unwrap_or(true)
                })
                .unwrap_or(false);

            // Determine the element type for Slice<E>
            let element_type = if is_inner_node {
                "u64".to_string()
            } else {
                // Extract the element type from vector<ElementType>
                let vector_start = type_str.find("vector<").unwrap_or(0);
                let element_type_start = vector_start + 7; // length of "vector<"

                // Find matching closing bracket for vector<...>
                let remaining = &type_str[element_type_start..];
                let mut depth = 1;
                let mut element_end = 0;
                for (i, c) in remaining.chars().enumerate() {
                    match c {
                        '<' => depth += 1,
                        '>' => {
                            depth -= 1;
                            if depth == 0 {
                                element_end = i;
                                break;
                            }
                        }
                        _ => {}
                    }
                }
                remaining[..element_end].to_string()
            };

            // Build the corrected type with Slice<Element>
            let slice_type = format!("{}::big_vector::Slice<{}>", DEEPBOOK_PACKAGE, element_type);

            // Get prefix (everything before "vector<")
            let vector_start = type_str.find("vector<").unwrap_or(0);
            let prefix = &type_str[..vector_start];

            // Get suffix (everything after "vector<...>")
            let element_type_start = vector_start + 7;
            let remaining = &type_str[element_type_start..];
            let mut depth = 1;
            let mut element_end = 0;
            for (i, c) in remaining.chars().enumerate() {
                match c {
                    '<' => depth += 1,
                    '>' => {
                        depth -= 1;
                        if depth == 0 {
                            element_end = i;
                            break;
                        }
                    }
                    _ => {}
                }
            }
            let suffix = &type_str[element_type_start + element_end + 1..];

            let corrected = format!("{}{}{}", prefix, slice_type, suffix);

            if is_inner_node {
                eprintln!("DEBUG: Inner node slice detected, using Slice<u64>");
            }

            tracing::debug!(
                "Corrected BigVector slice type: {} -> {}",
                type_str,
                corrected
            );

            corrected
        } else {
            type_str.to_string()
        }
    }

    /// Load a single object into the simulation environment
    fn load_object(&mut self, obj: &ExportedObject) -> Result<()> {
        // Convert object_json to BCS bytes using bytecode layouts
        let bcs_bytes = match self
            .bcs_converter
            .convert(&obj.object_type, &obj.object_json)
        {
            Ok(bytes) => bytes,
            Err(e) => {
                // Fallback to JSON serialization if conversion fails
                // This can happen for types not in the loaded bytecode
                tracing::warn!(
                    "BCS conversion failed for {} (type: {}), using JSON fallback: {}",
                    obj.object_id,
                    obj.object_type,
                    e
                );
                serde_json::to_vec(&obj.object_json)?
            }
        };

        let is_shared = obj.owner_type.as_deref() == Some("Shared");

        self.env.load_object_from_data(
            &obj.object_id,
            bcs_bytes,
            Some(&obj.object_type),
            is_shared,
            false, // not immutable
            obj.version,
        )?;

        Ok(())
    }

    /// Build orderbook by calling iter_orders for both bids and asks
    pub fn build_orderbook(
        &mut self,
        pool_id: PoolId,
        pool_object_id: &str,
        checkpoint: u64,
    ) -> Result<SandboxOrderbook> {
        if !self.packages_loaded {
            return Err(anyhow!("Packages not loaded. Call load_packages_* first"));
        }

        let (base_type, quote_type, base_decimals, quote_decimals) = match pool_id {
            PoolId::SuiUsdc => (SUI_TYPE, USDC_TYPE, 9u8, 6u8),
            PoolId::WalUsdc => (WAL_TYPE, USDC_TYPE, 9u8, 6u8),
            PoolId::DeepUsdc => (DEEP_TYPE, USDC_TYPE, 6u8, 6u8),
        };

        // Get bids
        let bid_orders = self.call_iter_orders(
            pool_object_id,
            base_type,
            quote_type,
            true, // bids
            1000, // limit
        )?;

        // Get asks
        let ask_orders = self.call_iter_orders(
            pool_object_id,
            base_type,
            quote_type,
            false, // asks
            1000,  // limit
        )?;

        // Aggregate to price levels
        let bids = Self::aggregate_orders(&bid_orders, true);
        let asks = Self::aggregate_orders(&ask_orders, false);

        Ok(SandboxOrderbook {
            pool_id,
            bids,
            asks,
            checkpoint,
            base_decimals,
            quote_decimals,
        })
    }

    /// Call deepbook::order_query::iter_orders using PTB
    fn call_iter_orders(
        &mut self,
        pool_object_id: &str,
        base_type: &str,
        quote_type: &str,
        bids: bool,
        limit: u64,
    ) -> Result<Vec<DecodedOrder>> {
        let deepbook_addr = AccountAddress::from_hex_literal(DEEPBOOK_PACKAGE)?;
        let pool_addr = AccountAddress::from_hex_literal(pool_object_id)?;

        // Parse type arguments
        let base_tag = TypeTag::from_str(base_type)?;
        let quote_tag = TypeTag::from_str(quote_type)?;

        // Get the pool object from cache or environment
        let (pool_bytes, pool_type, pool_version) = self
            .pool_cache
            .get(pool_object_id)
            .cloned()
            .or_else(|| {
                self.env.get_object(&pool_addr).map(|obj| {
                    let pool_type = build_pool_type_tag(base_type, quote_type).ok()?;
                    Some((obj.bcs_bytes.clone(), pool_type, obj.version))
                })?
            })
            .ok_or_else(|| anyhow!("Pool object {} not found", pool_object_id))?;

        // Build inputs for the PTB
        // Input 0: Pool object (shared, by reference)
        // Input 1: start_order_id (Option<u128>) = None
        // Input 2: end_order_id (Option<u128>) = None
        // Input 3: min_expire_timestamp (Option<u64>) = None
        // Input 4: limit (u64)
        // Input 5: bids (bool)
        let inputs = vec![
            InputValue::Object(ObjectInput::Shared {
                id: pool_addr,
                bytes: pool_bytes,
                type_tag: Some(pool_type),
                version: Some(pool_version),
                mutable: false, // Read-only access for view function
            }),
            InputValue::Pure(bcs::to_bytes(&Option::<u128>::None)?),
            InputValue::Pure(bcs::to_bytes(&Option::<u128>::None)?),
            InputValue::Pure(bcs::to_bytes(&Option::<u64>::None)?),
            InputValue::Pure(bcs::to_bytes(&limit)?),
            InputValue::Pure(bcs::to_bytes(&bids)?),
        ];

        let commands = vec![Command::MoveCall {
            package: deepbook_addr,
            module: Identifier::new("order_query")?,
            function: Identifier::new("iter_orders")?,
            type_args: vec![base_tag, quote_tag],
            args: vec![
                Argument::Input(0), // pool
                Argument::Input(1), // start_order_id
                Argument::Input(2), // end_order_id
                Argument::Input(3), // min_expire_timestamp
                Argument::Input(4), // limit
                Argument::Input(5), // bids
            ],
        }];

        let result = self.env.execute_ptb(inputs, commands);

        if !result.success {
            return Err(anyhow!(
                "iter_orders failed: {}",
                result
                    .raw_error
                    .unwrap_or_else(|| "Unknown error".to_string())
            ));
        }

        // Parse the return value (OrderPage) from effects
        // return_values is Vec<Vec<Vec<u8>>>:
        //   - outer: one element per command
        //   - middle: one element per return value from that command
        //   - inner: the actual BCS bytes
        let return_bytes = result
            .effects
            .as_ref()
            .and_then(|effects| {
                // Get command 0's return values
                effects.return_values.first()
            })
            .and_then(|cmd_returns| {
                // Get the first return value from command 0
                cmd_returns.first().cloned()
            })
            .ok_or_else(|| anyhow!("No return values from iter_orders"))?;

        self.parse_order_page(&return_bytes, bids)
    }

    /// Parse OrderPage from BCS bytes
    fn parse_order_page(&self, bytes: &[u8], is_bid: bool) -> Result<Vec<DecodedOrder>> {
        // OrderPage struct layout:
        // - orders: vector<Order>
        // - has_next_page: bool
        //
        // Order struct layout:
        // - balance_manager_id: ID (32 bytes)
        // - order_id: u128 (16 bytes)
        // - client_order_id: u64 (8 bytes)
        // - quantity: u64 (8 bytes)
        // - filled_quantity: u64 (8 bytes)
        // - fee_is_deep: bool (1 byte)
        // - order_deep_price: OrderDeepPrice { asset_is_base: bool, deep_per_asset: u64 }
        // - epoch: u64 (8 bytes)
        // - status: u8 (1 byte)
        // - expire_timestamp: u64 (8 bytes)

        let mut orders = Vec::new();
        let mut cursor = std::io::Cursor::new(bytes);

        // Read vector length (ULEB128)
        let len = read_uleb128(&mut cursor)?;

        for _ in 0..len {
            // Skip balance_manager_id (32 bytes)
            let mut id_bytes = [0u8; 32];
            std::io::Read::read_exact(&mut cursor, &mut id_bytes)?;

            // Read order_id (u128, little-endian)
            let mut order_id_bytes = [0u8; 16];
            std::io::Read::read_exact(&mut cursor, &mut order_id_bytes)?;
            let order_id = u128::from_le_bytes(order_id_bytes);

            // Read client_order_id (u64)
            let mut client_order_id_bytes = [0u8; 8];
            std::io::Read::read_exact(&mut cursor, &mut client_order_id_bytes)?;

            // Read quantity (u64)
            let mut quantity_bytes = [0u8; 8];
            std::io::Read::read_exact(&mut cursor, &mut quantity_bytes)?;
            let quantity = u64::from_le_bytes(quantity_bytes);

            // Read filled_quantity (u64)
            let mut filled_quantity_bytes = [0u8; 8];
            std::io::Read::read_exact(&mut cursor, &mut filled_quantity_bytes)?;
            let filled_quantity = u64::from_le_bytes(filled_quantity_bytes);

            // Extract price from order_id using DeepBook's encoding
            // Bit 127 = side, Bits 64-126 = price, Bits 0-63 = sequence
            let price = ((order_id >> 64) & ((1u128 << 63) - 1)) as u64;

            // Read fee_is_deep (1 byte)
            let mut fee_is_deep_byte = [0u8; 1];
            std::io::Read::read_exact(&mut cursor, &mut fee_is_deep_byte)?;

            // Read order_deep_price (1 byte bool + 8 bytes u64)
            let mut _asset_is_base = [0u8; 1];
            std::io::Read::read_exact(&mut cursor, &mut _asset_is_base)?;
            let mut _deep_per_asset = [0u8; 8];
            std::io::Read::read_exact(&mut cursor, &mut _deep_per_asset)?;

            // Read epoch (u64)
            let mut _epoch_bytes = [0u8; 8];
            std::io::Read::read_exact(&mut cursor, &mut _epoch_bytes)?;

            // Read status (u8)
            let mut _status_byte = [0u8; 1];
            std::io::Read::read_exact(&mut cursor, &mut _status_byte)?;

            // Read expire_timestamp (u64)
            let mut expire_timestamp_bytes = [0u8; 8];
            std::io::Read::read_exact(&mut cursor, &mut expire_timestamp_bytes)?;
            let expire_timestamp = u64::from_le_bytes(expire_timestamp_bytes);

            orders.push(DecodedOrder {
                order_id,
                price,
                quantity,
                filled_quantity,
                is_bid,
                expire_timestamp,
            });
        }

        Ok(orders)
    }

    /// Aggregate orders into price levels
    fn aggregate_orders(orders: &[DecodedOrder], is_bid: bool) -> Vec<PriceLevel> {
        let mut levels: HashMap<u64, (u64, usize)> = HashMap::new();

        for order in orders {
            let remaining = order.remaining_quantity();
            if remaining == 0 {
                continue;
            }

            let entry = levels.entry(order.price).or_insert((0, 0));
            entry.0 += remaining;
            entry.1 += 1;
        }

        let mut result: Vec<PriceLevel> = levels
            .into_iter()
            .map(|(price, (total_quantity, order_count))| PriceLevel {
                price,
                total_quantity,
                order_count,
            })
            .collect();

        // Sort: bids descending, asks ascending
        if is_bid {
            result.sort_by(|a, b| b.price.cmp(&a.price));
        } else {
            result.sort_by(|a, b| a.price.cmp(&b.price));
        }

        result
    }
}

/// Build Pool<BaseAsset, QuoteAsset> TypeTag
fn build_pool_type_tag(base_type: &str, quote_type: &str) -> Result<TypeTag> {
    let base_tag = TypeTag::from_str(base_type)?;
    let quote_tag = TypeTag::from_str(quote_type)?;

    Ok(TypeTag::Struct(Box::new(StructTag {
        address: AccountAddress::from_hex_literal(DEEPBOOK_PACKAGE)?,
        module: Identifier::new("pool")?,
        name: Identifier::new("Pool")?,
        type_params: vec![base_tag, quote_tag],
    })))
}

/// Read ULEB128 encoded integer
fn read_uleb128(cursor: &mut std::io::Cursor<&[u8]>) -> Result<usize> {
    let mut result = 0usize;
    let mut shift = 0;

    loop {
        let mut byte = [0u8; 1];
        std::io::Read::read_exact(cursor, &mut byte)?;
        let b = byte[0];

        result |= ((b & 0x7f) as usize) << shift;

        if b & 0x80 == 0 {
            break;
        }

        shift += 7;
        if shift >= 64 {
            return Err(anyhow!("ULEB128 overflow"));
        }
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_price_extraction() {
        // Test order ID encoding
        // Bid: bit 127 = 0, bits 64-126 = price, bits 0-63 = seq
        let price = 1_150_000u64; // $1.15
        let seq = 12345u64;
        let order_id: u128 = ((price as u128) << 64) | (seq as u128);

        let extracted_price = ((order_id >> 64) & ((1u128 << 63) - 1)) as u64;
        assert_eq!(extracted_price, price);
    }

    #[test]
    fn test_aggregate_orders() {
        let orders = vec![
            DecodedOrder {
                order_id: 0,
                price: 1_000_000,
                quantity: 100,
                filled_quantity: 0,
                is_bid: true,
                expire_timestamp: 0,
            },
            DecodedOrder {
                order_id: 1,
                price: 1_000_000, // Same price
                quantity: 200,
                filled_quantity: 50,
                is_bid: true,
                expire_timestamp: 0,
            },
            DecodedOrder {
                order_id: 2,
                price: 999_000, // Lower price
                quantity: 300,
                filled_quantity: 0,
                is_bid: true,
                expire_timestamp: 0,
            },
        ];

        let levels = OrderbookBuilder::aggregate_orders(&orders, true);

        assert_eq!(levels.len(), 2);
        // First level should be highest price (1.00)
        assert_eq!(levels[0].price, 1_000_000);
        assert_eq!(levels[0].total_quantity, 250); // 100 + (200-50)
        assert_eq!(levels[0].order_count, 2);
        // Second level should be lower price (0.999)
        assert_eq!(levels[1].price, 999_000);
        assert_eq!(levels[1].total_quantity, 300);
    }
}
