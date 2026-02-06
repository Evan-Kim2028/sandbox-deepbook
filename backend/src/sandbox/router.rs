//! Cross-Pool Router using MoveVM
//!
//! Dedicated thread that owns a SimulationEnvironment with all pool states loaded.
//! Compiles and deploys the router Move contract, then executes `quote_two_hop`
//! PTBs on demand via mpsc channels.

use anyhow::{anyhow, Result};
use move_core_types::account_address::AccountAddress;
use move_core_types::identifier::Identifier;
use move_core_types::language_storage::TypeTag;
use std::collections::HashMap;
use std::path::Path;
use std::str::FromStr;
use std::sync::mpsc;
use tokio::sync::oneshot;
use tracing;

use sui_sandbox_core::fetcher::GrpcFetcher;
use sui_sandbox_core::ptb::{Argument, Command, InputValue, ObjectInput};
use sui_sandbox_core::simulation::state::FetcherConfig;
use sui_sandbox_core::simulation::SimulationEnvironment;

use super::orderbook_builder::build_pool_type_tag;
use super::snowflake_bcs::JsonToBcsConverter;
use super::state_loader::{DeepBookConfig, ExportedObject, PoolId, StateLoader};

// DeepBook V3 Package
const DEEPBOOK_PACKAGE: &str =
    "0x2c8d603bc51326b8c13cef9dd07031a408a48dddb541963357661df5d3204809";

// Type tags for assets
const SUI_TYPE: &str = "0x2::sui::SUI";
const USDC_TYPE: &str =
    "0xdba34672e30cb065b1f93e3ab55318768fd6fef66c15942c9f7cb846e2f900e7::usdc::USDC";
const WAL_TYPE: &str =
    "0x356a26eb9e012a68958082340d4c4116e7f55615cf27affcff209cf0ae544f59::wal::WAL";
const DEEP_TYPE: &str =
    "0xdeeb7a4662eec9f2f3def03fb937a663dddaa2e215b8078a284d026b7946c270::deep::DEEP";

// Synthetic addresses for router deployment
const ROUTER_PACKAGE_ADDR: &str = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const CLOCK_OBJECT_ID: &str = "0x6";

/// Result of a two-hop quote from the MoveVM router
#[derive(Debug, Clone)]
pub struct TwoHopQuote {
    pub final_output: u64,
    pub intermediate_amount: u64,
}

/// Request sent to the router thread
struct RouterRequest {
    from_pool: PoolId,
    to_pool: PoolId,
    input_amount: u64,
    response_tx: oneshot::Sender<Result<TwoHopQuote>>,
}

/// Handle for communicating with the router thread (Send+Sync)
#[derive(Clone)]
pub struct RouterHandle {
    tx: mpsc::Sender<RouterRequest>,
}

impl RouterHandle {
    /// Request a two-hop quote from the router thread
    pub async fn quote_two_hop(
        &self,
        from_pool: PoolId,
        to_pool: PoolId,
        input_amount: u64,
    ) -> Result<TwoHopQuote> {
        let (response_tx, response_rx) = oneshot::channel();

        self.tx
            .send(RouterRequest {
                from_pool,
                to_pool,
                input_amount,
                response_tx,
            })
            .map_err(|_| anyhow!("Router thread has shut down"))?;

        response_rx
            .await
            .map_err(|_| anyhow!("Router thread dropped response channel"))?
    }
}

/// Spawn the router thread and return a handle for communication.
///
/// The thread:
/// 1. Creates a SimulationEnvironment
/// 2. Loads all packages via gRPC
/// 3. Loads all pool states from JSONL files
/// 4. Creates a synthetic Clock object
/// 5. Compiles and deploys the router contract
/// 6. Signals ready
/// 7. Loops processing quote requests
pub fn spawn_router_thread(
    pool_files: Vec<(PoolId, String)>,
) -> (RouterHandle, oneshot::Receiver<Result<()>>) {
    let (tx, rx) = mpsc::channel::<RouterRequest>();
    let (ready_tx, ready_rx) = oneshot::channel::<Result<()>>();

    std::thread::spawn(move || {
        router_thread_main(rx, ready_tx, pool_files);
    });

    (RouterHandle { tx }, ready_rx)
}

fn router_thread_main(
    rx: mpsc::Receiver<RouterRequest>,
    ready_tx: oneshot::Sender<Result<()>>,
    pool_files: Vec<(PoolId, String)>,
) {
    let result = setup_router_env(&pool_files);

    match result {
        Ok(mut env_state) => {
            let _ = ready_tx.send(Ok(()));
            tracing::info!("Router thread ready, processing quote requests");

            // Process requests
            while let Ok(req) = rx.recv() {
                let result = execute_quote(&mut env_state, req.from_pool, req.to_pool, req.input_amount);
                let _ = req.response_tx.send(result);
            }

            tracing::info!("Router thread shutting down (channel closed)");
        }
        Err(e) => {
            tracing::error!("Router thread setup failed: {}", e);
            let _ = ready_tx.send(Err(e));
        }
    }
}

/// Internal state for the router environment
struct RouterEnvState {
    env: SimulationEnvironment,
    pool_cache: HashMap<PoolId, PoolCacheEntry>,
    router_deployed: bool,
}

struct PoolCacheEntry {
    pool_addr: AccountAddress,
    pool_bytes: Vec<u8>,
    pool_type: TypeTag,
    pool_version: u64,
}

fn setup_router_env(pool_files: &[(PoolId, String)]) -> Result<RouterEnvState> {
    tracing::info!("Router thread: creating SimulationEnvironment...");
    let mut env = SimulationEnvironment::new()?;
    let mut bcs_converter = JsonToBcsConverter::new();

    // Create a tokio runtime for async gRPC calls
    let rt = tokio::runtime::Runtime::new()?;

    // Load packages via gRPC
    tracing::info!("Router thread: loading packages via gRPC...");
    let grpc = rt.block_on(async { sui_transport::grpc::GrpcClient::mainnet().await })?;

    // Configure auto-fetch for missing packages
    let fetcher = GrpcFetcher::mainnet();
    let config = FetcherConfig::mainnet();
    env.set_fetcher(Box::new(fetcher));
    env.set_fetcher_config(config);

    let packages_to_fetch = [
        ("0x1", "Move Stdlib"),
        ("0x2", "Sui Framework"),
        (DEEPBOOK_PACKAGE, "DeepBook V3"),
        (USDC_TYPE.split("::").next().unwrap(), "USDC"),
        (WAL_TYPE.split("::").next().unwrap(), "WAL"),
        (DEEP_TYPE.split("::").next().unwrap(), "DEEP"),
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
        if let Ok(Some(obj)) = rt.block_on(grpc.get_object(pkg_id)) {
            if let Some(modules) = obj.package_modules {
                let bytecode_list: Vec<Vec<u8>> =
                    modules.iter().map(|(_, bytes)| bytes.clone()).collect();
                if let Err(e) = bcs_converter.add_modules_from_bytes(&bytecode_list) {
                    tracing::warn!("Router: failed to add {} to BCS converter: {}", name, e);
                }
                env.deploy_package_at_address(pkg_id, modules)?;
                tracing::info!("Router: loaded {} ({})", name, pkg_id);
            }
        }
    }

    // Load all pool states
    let mut pool_cache = HashMap::new();
    for (pool_id, file_path) in pool_files {
        let path = Path::new(file_path);
        if !path.exists() {
            tracing::warn!("Router: skipping {} - file not found: {}", pool_id.display_name(), file_path);
            continue;
        }

        let config = DeepBookConfig::for_pool(*pool_id);
        let pool_wrapper_id = config.pool_wrapper.clone();
        let mut loader = StateLoader::with_config(config);
        loader.load_from_file(path).map_err(|e| anyhow!("Router: failed to load {}: {}", file_path, e))?;

        // Load objects into simulation environment
        for obj in loader.all_objects() {
            if let Some(owner_addr) = &obj.owner_address {
                if obj.object_type.contains("dynamic_field::Field") {
                    load_dynamic_field_for_router(&mut env, &mut bcs_converter, obj, owner_addr)?;
                    continue;
                }
            }
            load_object_for_router(&mut env, &mut bcs_converter, obj)?;
        }

        // Cache pool entry for PTB construction
        if let Some(pool_obj) = loader.get_object(&pool_wrapper_id) {
            let (base_type, quote_type) = match pool_id {
                PoolId::SuiUsdc => (SUI_TYPE, USDC_TYPE),
                PoolId::WalUsdc => (WAL_TYPE, USDC_TYPE),
                PoolId::DeepUsdc => (DEEP_TYPE, USDC_TYPE),
            };

            let pool_type = build_pool_type_tag(base_type, quote_type)?;
            let bcs_bytes = bcs_converter
                .convert(&pool_obj.object_type, &pool_obj.object_json)
                .unwrap_or_else(|_| serde_json::to_vec(&pool_obj.object_json).unwrap());

            let pool_addr = AccountAddress::from_hex_literal(&pool_wrapper_id)?;
            pool_cache.insert(
                *pool_id,
                PoolCacheEntry {
                    pool_addr,
                    pool_bytes: bcs_bytes,
                    pool_type,
                    pool_version: pool_obj.version,
                },
            );
        }

        tracing::info!("Router: loaded {} pool state", pool_id.display_name());
    }

    // Create synthetic Clock object at 0x6
    create_clock_object(&mut env)?;

    // Compile and deploy router contract
    deploy_router_contract(&mut env)?;

    Ok(RouterEnvState {
        env,
        pool_cache,
        router_deployed: true,
    })
}

/// Create a synthetic Clock object at address 0x6
fn create_clock_object(env: &mut SimulationEnvironment) -> Result<()> {
    // Clock struct in BCS: UID (32 bytes) + timestamp_ms (u64)
    // UID is the object ID padded to 32 bytes
    let clock_addr = AccountAddress::from_hex_literal(CLOCK_OBJECT_ID)?;
    let mut bcs_bytes = Vec::new();
    bcs_bytes.extend_from_slice(clock_addr.as_ref()); // UID = 32 bytes
    let timestamp_ms: u64 = 1_770_000_000_000; // ~2026 timestamp
    bcs_bytes.extend_from_slice(&timestamp_ms.to_le_bytes());

    env.load_object_from_data(
        CLOCK_OBJECT_ID,
        bcs_bytes,
        Some("0x2::clock::Clock"),
        true,  // shared
        false, // not immutable
        1,     // version
    )?;

    tracing::info!("Router: created synthetic Clock at 0x6");
    Ok(())
}

/// Deploy the router contract from compiled bytecode
fn deploy_router_contract(env: &mut SimulationEnvironment) -> Result<()> {
    // Build the router contract
    // Try both relative paths (from backend/ and from project root)
    let router_dir = if Path::new("./contracts/router").exists() {
        Path::new("./contracts/router")
    } else if Path::new("../contracts/router").exists() {
        Path::new("../contracts/router")
    } else {
        return Err(anyhow!(
            "Router contract directory not found at ./contracts/router or ../contracts/router"
        ));
    };

    tracing::info!("Router: compiling router contract...");

    // Run `sui move build` to compile
    let output = std::process::Command::new("sui")
        .args(["move", "build", "--skip-fetch-latest-git-deps"])
        .current_dir(router_dir)
        .output();

    match output {
        Ok(out) if out.status.success() => {
            tracing::info!("Router: contract compiled successfully");
        }
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr);
            // Try without --skip-fetch-latest-git-deps on first build
            tracing::info!("Router: retrying compilation with fresh deps...");
            let out2 = std::process::Command::new("sui")
                .args(["move", "build"])
                .current_dir(router_dir)
                .output()
                .map_err(|e| anyhow!("Failed to run sui move build: {}", e))?;

            if !out2.status.success() {
                let stderr2 = String::from_utf8_lossy(&out2.stderr);
                return Err(anyhow!(
                    "Router contract compilation failed:\n{}\n{}",
                    stderr,
                    stderr2
                ));
            }
            tracing::info!("Router: contract compiled successfully (fresh deps)");
        }
        Err(e) => {
            return Err(anyhow!("Failed to run sui move build: {}", e));
        }
    }

    // Read compiled bytecode from build directory
    let build_dir = router_dir.join("build/DeepBookRouter/bytecode_modules");
    let mut modules = Vec::new();

    if build_dir.exists() {
        for entry in std::fs::read_dir(&build_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "mv") {
                let module_name = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("unknown")
                    .to_string();
                let bytecode = std::fs::read(&path)?;
                tracing::info!(
                    "Router: loaded module '{}' ({} bytes)",
                    module_name,
                    bytecode.len()
                );
                modules.push((module_name, bytecode));
            }
        }
    }

    if modules.is_empty() {
        return Err(anyhow!(
            "No compiled modules found in {}",
            build_dir.display()
        ));
    }

    // Deploy at a synthetic address
    env.deploy_package_at_address(ROUTER_PACKAGE_ADDR, modules)?;
    tracing::info!("Router: deployed router contract at {}", ROUTER_PACKAGE_ADDR);

    Ok(())
}

/// Execute a two-hop quote via the MoveVM router contract
fn execute_quote(
    state: &mut RouterEnvState,
    from_pool: PoolId,
    to_pool: PoolId,
    input_amount: u64,
) -> Result<TwoHopQuote> {
    if !state.router_deployed {
        return Err(anyhow!("Router contract not deployed"));
    }

    // Get pool cache entries
    let from_entry = state.pool_cache.get(&from_pool).ok_or_else(|| {
        anyhow!("Pool {} not loaded in router", from_pool.display_name())
    })?;
    let to_entry = state.pool_cache.get(&to_pool).ok_or_else(|| {
        anyhow!("Pool {} not loaded in router", to_pool.display_name())
    })?;

    // Determine type args: A (base of from_pool), Q (USDC), B (base of to_pool)
    let (a_type, q_type, b_type) = resolve_two_hop_types(from_pool, to_pool)?;

    let a_tag = TypeTag::from_str(a_type)?;
    let q_tag = TypeTag::from_str(q_type)?;
    let b_tag = TypeTag::from_str(b_type)?;

    let router_addr = AccountAddress::from_hex_literal(ROUTER_PACKAGE_ADDR)?;
    let clock_addr = AccountAddress::from_hex_literal(CLOCK_OBJECT_ID)?;

    // Build Clock object bytes (UID + timestamp_ms)
    let mut clock_bytes = Vec::new();
    clock_bytes.extend_from_slice(clock_addr.as_ref());
    let timestamp_ms: u64 = 1_770_000_000_000;
    clock_bytes.extend_from_slice(&timestamp_ms.to_le_bytes());

    let inputs = vec![
        // Input 0: Pool<A, Q> (shared, immutable ref)
        InputValue::Object(ObjectInput::Shared {
            id: from_entry.pool_addr,
            bytes: from_entry.pool_bytes.clone(),
            type_tag: Some(from_entry.pool_type.clone()),
            version: Some(from_entry.pool_version),
            mutable: false,
        }),
        // Input 1: Pool<B, Q> (shared, immutable ref)
        InputValue::Object(ObjectInput::Shared {
            id: to_entry.pool_addr,
            bytes: to_entry.pool_bytes.clone(),
            type_tag: Some(to_entry.pool_type.clone()),
            version: Some(to_entry.pool_version),
            mutable: false,
        }),
        // Input 2: input_amount (pure u64)
        InputValue::Pure(bcs::to_bytes(&input_amount)?),
        // Input 3: Clock at 0x6 (shared, immutable ref)
        InputValue::Object(ObjectInput::Shared {
            id: clock_addr,
            bytes: clock_bytes,
            type_tag: Some(TypeTag::from_str("0x2::clock::Clock")?),
            version: Some(1),
            mutable: false,
        }),
    ];

    let commands = vec![Command::MoveCall {
        package: router_addr,
        module: Identifier::new("router")?,
        function: Identifier::new("quote_two_hop")?,
        type_args: vec![a_tag, q_tag, b_tag],
        args: vec![
            Argument::Input(0), // pool_aq
            Argument::Input(1), // pool_bq
            Argument::Input(2), // input_amount
            Argument::Input(3), // clock
        ],
    }];

    let result = state.env.execute_ptb(inputs, commands);

    if !result.success {
        return Err(anyhow!(
            "quote_two_hop failed: {}",
            result.raw_error.unwrap_or_else(|| "Unknown error".to_string())
        ));
    }

    // Parse return values: (u64, u64) = (final_output, intermediate_amount)
    let return_values = result
        .effects
        .as_ref()
        .and_then(|effects| effects.return_values.first())
        .ok_or_else(|| anyhow!("No return values from quote_two_hop"))?;

    // First return value: base_out (u64)
    let final_output = if let Some(bytes) = return_values.first() {
        if bytes.len() >= 8 {
            u64::from_le_bytes(bytes[..8].try_into()?)
        } else {
            return Err(anyhow!("Invalid final_output bytes"));
        }
    } else {
        return Err(anyhow!("Missing final_output return value"));
    };

    // Second return value: quote_out (u64)
    let intermediate_amount = if let Some(bytes) = return_values.get(1) {
        if bytes.len() >= 8 {
            u64::from_le_bytes(bytes[..8].try_into()?)
        } else {
            return Err(anyhow!("Invalid intermediate_amount bytes"));
        }
    } else {
        return Err(anyhow!("Missing intermediate_amount return value"));
    };

    Ok(TwoHopQuote {
        final_output,
        intermediate_amount,
    })
}

/// Resolve type arguments for a two-hop swap: A -> USDC -> B
fn resolve_two_hop_types(from_pool: PoolId, to_pool: PoolId) -> Result<(&'static str, &'static str, &'static str)> {
    let a_type = match from_pool {
        PoolId::SuiUsdc => SUI_TYPE,
        PoolId::WalUsdc => WAL_TYPE,
        PoolId::DeepUsdc => DEEP_TYPE,
    };

    let b_type = match to_pool {
        PoolId::SuiUsdc => SUI_TYPE,
        PoolId::WalUsdc => WAL_TYPE,
        PoolId::DeepUsdc => DEEP_TYPE,
    };

    Ok((a_type, USDC_TYPE, b_type))
}

// Helper functions that mirror OrderbookBuilder's object loading

fn load_object_for_router(
    env: &mut SimulationEnvironment,
    bcs_converter: &mut JsonToBcsConverter,
    obj: &ExportedObject,
) -> Result<()> {
    let bcs_bytes = bcs_converter
        .convert(&obj.object_type, &obj.object_json)
        .unwrap_or_else(|_| serde_json::to_vec(&obj.object_json).unwrap_or_default());

    let is_shared = obj.owner_type.as_deref() == Some("Shared");

    env.load_object_from_data(
        &obj.object_id,
        bcs_bytes,
        Some(&obj.object_type),
        is_shared,
        false,
        obj.version,
    )?;

    Ok(())
}

fn load_dynamic_field_for_router(
    env: &mut SimulationEnvironment,
    bcs_converter: &mut JsonToBcsConverter,
    obj: &ExportedObject,
    parent_addr: &str,
) -> Result<()> {
    let parent_id = AccountAddress::from_hex_literal(parent_addr)?;
    let child_id = AccountAddress::from_hex_literal(&obj.object_id)?;

    let corrected_type = correct_bigvector_slice_type(&obj.object_type, &obj.object_json);

    let full_type_tag = SimulationEnvironment::parse_type_string(&corrected_type)
        .ok_or_else(|| anyhow!("Failed to parse field type: {}", corrected_type))?;

    let bcs_bytes = bcs_converter
        .convert(&corrected_type, &obj.object_json)
        .unwrap_or_else(|_| serde_json::to_vec(&obj.object_json).unwrap_or_default());

    env.set_dynamic_field(parent_id, child_id, full_type_tag, bcs_bytes);

    Ok(())
}

/// Correct BigVector slice types (same logic as OrderbookBuilder)
fn correct_bigvector_slice_type(type_str: &str, json: &serde_json::Value) -> String {
    if type_str.contains("::dynamic_field::Field<u64, vector<")
        && type_str.contains(DEEPBOOK_PACKAGE)
    {
        let is_inner_node = json
            .get("value")
            .and_then(|v| v.get("vals"))
            .and_then(|vals| vals.as_array())
            .map(|arr| arr.first().map(|v| v.is_string()).unwrap_or(true))
            .unwrap_or(false);

        let element_type = if is_inner_node {
            "u64".to_string()
        } else {
            let vector_start = type_str.find("vector<").unwrap_or(0);
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
            remaining[..element_end].to_string()
        };

        let slice_type = format!("{}::big_vector::Slice<{}>", DEEPBOOK_PACKAGE, element_type);
        let vector_start = type_str.find("vector<").unwrap_or(0);
        let prefix = &type_str[..vector_start];
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
        format!("{}{}{}", prefix, slice_type, suffix)
    } else {
        type_str.to_string()
    }
}
