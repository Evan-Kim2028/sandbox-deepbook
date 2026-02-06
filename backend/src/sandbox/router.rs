//! Quote router using MoveVM
//!
//! Dedicated thread that owns a SimulationEnvironment with all pool states loaded.
//! Executes DeepBook quote PTBs (single-hop) and router PTBs (two-hop) on demand
//! via mpsc channels.

use anyhow::{anyhow, Result};
use move_core_types::account_address::AccountAddress;
use move_core_types::identifier::Identifier;
use move_core_types::language_storage::TypeTag;
use serde::Serialize;
use serde_json::json;
use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::mpsc;
use tokio::sync::oneshot;
use tracing;

use sui_sandbox_core::fetcher::GrpcFetcher;
use sui_sandbox_core::ptb::{Argument, Command, InputValue, ObjectInput};
use sui_sandbox_core::simulation::state::FetcherConfig;
use sui_sandbox_core::simulation::SimulationEnvironment;
use sui_sandbox_core::tx_replay::derive_dynamic_field_id;
use sui_transport::grpc::{GrpcObject, GrpcOwner};

use super::orderbook_builder::build_pool_type_tag;
use super::snowflake_bcs::JsonToBcsConverter;
use super::state_loader::{DeepBookConfig, ExportedObject, PoolId, StateLoader};

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
const DEBUG_TYPE: &str =
    "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa::debug_token::DEBUG_TOKEN";
const DEBUG_TREASURY_TYPE: &str =
    "0x2::coin::TreasuryCap<0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa::debug_token::DEBUG_TOKEN>";
const DEEPBOOK_REGISTRY_ID: &str =
    "0xaf16199a2dff736e9f07a845f23c5da6df6f756eddb631aed9d24a93efc4549d";
const COIN_REGISTRY_OBJECT_ID: &str = "0xc";

// Synthetic addresses for router deployment
const ROUTER_PACKAGE_ADDR: &str =
    "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const CLOCK_OBJECT_ID: &str = "0x6";
const SUI_FRAMEWORK_PACKAGE: &str = "0x2";
const OBJECT_ID_TYPE: &str = "0x2::object::ID";
const DEBUG_ADMIN_CAP_ID: &str =
    "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaab";
const DEBUG_POOL_TICK_SIZE: u64 = 1_000;
const DEBUG_POOL_LOT_SIZE: u64 = 1_000;
const DEBUG_POOL_MIN_SIZE: u64 = 10_000;
const DEBUG_POOL_WHITELISTED: bool = true;
const DEBUG_POOL_BID_PRICE: u64 = 900_000; // $0.90
const DEBUG_POOL_ASK_PRICE: u64 = 1_100_000; // $1.10
const DEBUG_POOL_BID_QTY: u64 = 100_000_000_000; // 100 DBG @ 9 decimals
const DEBUG_POOL_ASK_QTY: u64 = 100_000_000_000; // 100 DBG @ 9 decimals
const DEBUG_POOL_USDC_LIQUIDITY: u64 = 200_000_000; // 200 USDC
const DEBUG_POOL_BASE_LIQUIDITY: u64 = 200_000_000_000; // 200 DBG
const DEBUG_POOL_DEEP_FEE_BUDGET: u64 = 100_000_000; // 100 DEEP
const DEBUG_POOL_PAY_WITH_DEEP: bool = false;
const RESERVE_COIN_SEED_AMOUNT: u64 = 100_000_000_000_000_000; // shared VM reserve per coin type
const MAINNET_RESERVE_SCAN_WINDOW: u64 = 150;
const SYNTHETIC_CLOCK_START_MS: u64 = 1_770_000_000_000; // ~2026 timestamp
const SYNTHETIC_CLOCK_STEP_MS: u64 = 61_000; // > DeepBook min 60s spacing for deep_price points
const DEBUG_ORDER_EXPIRY_TTL_MS: u64 = 86_400_000; // 1 day
const DEBUG_POOL_MAKER_SENDER: &str =
    "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

/// Result of a two-hop quote from the MoveVM router
#[derive(Debug, Clone)]
pub struct TwoHopQuote {
    pub final_output: u64,
    pub intermediate_amount: u64,
}

/// Result of a single-hop quote from MoveVM DeepBook pool calls
#[derive(Debug, Clone)]
pub struct SingleHopQuote {
    pub output_amount: u64,
}

/// Event emitted during swap execution (BCS payload is hex-encoded).
#[derive(Debug, Clone)]
pub struct SwapEvent {
    pub event_type: String,
    pub data_hex: String,
}

/// Result of a single-hop swap executed in MoveVM.
#[derive(Debug, Clone)]
pub struct SingleHopSwapResult {
    pub output_amount: u64,
    pub input_refund: u64,
    pub deep_refund: u64,
    pub gas_used: u64,
    pub events: Vec<SwapEvent>,
}

/// Result of a two-hop swap executed in MoveVM.
#[derive(Debug, Clone)]
pub struct TwoHopSwapResult {
    pub output_amount: u64,
    pub intermediate_amount: u64,
    pub input_refund: u64,
    pub quote_refund: u64,
    pub deep_refund: u64,
    pub gas_used: u64,
    pub events: Vec<SwapEvent>,
}

/// Result of VM-backed faucet execution.
#[derive(Debug, Clone)]
pub struct VmFaucetResult {
    pub amount: u64,
    pub gas_used: u64,
    pub created_objects: Vec<String>,
    pub events: Vec<SwapEvent>,
}

/// Metadata for the on-demand debug pool.
#[derive(Debug, Clone)]
pub struct DebugPoolInfo {
    pub pool_object_id: String,
    pub token_symbol: String,
    pub token_type: String,
    pub config: DebugPoolCreateConfig,
}

/// Configurable parameters for creating/seeding the debug pool in local VM.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DebugPoolCreateConfig {
    pub token_symbol: String,
    pub token_name: String,
    pub token_description: String,
    pub token_icon_url: String,
    pub token_decimals: u8,
    pub tick_size: u64,
    pub lot_size: u64,
    pub min_size: u64,
    pub whitelisted_pool: bool,
    pub pay_with_deep: bool,
    pub bid_price: u64,
    pub ask_price: u64,
    pub bid_quantity: u64,
    pub ask_quantity: u64,
    pub base_liquidity: u64,
    pub quote_liquidity: u64,
    pub deep_fee_budget: u64,
}

impl Default for DebugPoolCreateConfig {
    fn default() -> Self {
        Self {
            token_symbol: "DBG".to_string(),
            token_name: "Debug Token".to_string(),
            token_description: "Local VM debug token for DeepBook sandbox flows".to_string(),
            token_icon_url: String::new(),
            token_decimals: 9,
            tick_size: DEBUG_POOL_TICK_SIZE,
            lot_size: DEBUG_POOL_LOT_SIZE,
            min_size: DEBUG_POOL_MIN_SIZE,
            whitelisted_pool: DEBUG_POOL_WHITELISTED,
            pay_with_deep: DEBUG_POOL_PAY_WITH_DEEP,
            bid_price: DEBUG_POOL_BID_PRICE,
            ask_price: DEBUG_POOL_ASK_PRICE,
            bid_quantity: DEBUG_POOL_BID_QTY,
            ask_quantity: DEBUG_POOL_ASK_QTY,
            base_liquidity: DEBUG_POOL_BASE_LIQUIDITY,
            quote_liquidity: DEBUG_POOL_USDC_LIQUIDITY,
            deep_fee_budget: DEBUG_POOL_DEEP_FEE_BUDGET,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct RouterSharedObjectCheck {
    pub name: String,
    pub object_id: String,
    pub present: bool,
    pub is_shared: bool,
    pub version: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RouterReserveCoinCheck {
    pub coin_type: String,
    pub object_id: Option<String>,
    pub present: bool,
    pub version: Option<u64>,
    pub value: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RouterStartupCheckReport {
    pub ok: bool,
    pub checked_at_unix_ms: u64,
    pub router_package_deployed: bool,
    pub router_health_check_passed: bool,
    pub shared_objects: Vec<RouterSharedObjectCheck>,
    pub reserve_coins: Vec<RouterReserveCoinCheck>,
    pub errors: Vec<String>,
}

impl Default for RouterStartupCheckReport {
    fn default() -> Self {
        Self {
            ok: false,
            checked_at_unix_ms: 0,
            router_package_deployed: false,
            router_health_check_passed: false,
            shared_objects: Vec::new(),
            reserve_coins: Vec::new(),
            errors: Vec::new(),
        }
    }
}

/// Request sent to the router thread
enum RouterRequest {
    TwoHop {
        from_pool: PoolId,
        to_pool: PoolId,
        input_amount: u64,
        response_tx: oneshot::Sender<Result<TwoHopQuote>>,
    },
    SingleHop {
        pool_id: PoolId,
        input_amount: u64,
        is_sell_base: bool,
        response_tx: oneshot::Sender<Result<SingleHopQuote>>,
    },
    ExecuteSingleHop {
        pool_id: PoolId,
        input_amount: u64,
        deep_amount: u64,
        is_sell_base: bool,
        response_tx: oneshot::Sender<Result<SingleHopSwapResult>>,
    },
    ExecuteTwoHop {
        from_pool: PoolId,
        to_pool: PoolId,
        input_amount: u64,
        deep_amount: u64,
        response_tx: oneshot::Sender<Result<TwoHopSwapResult>>,
    },
    EnsureDebugPool {
        response_tx: oneshot::Sender<Result<DebugPoolInfo>>,
    },
    EnsureDebugPoolWithConfig {
        config: DebugPoolCreateConfig,
        response_tx: oneshot::Sender<Result<DebugPoolInfo>>,
    },
    VmFaucet {
        coin_type: String,
        amount: u64,
        response_tx: oneshot::Sender<Result<VmFaucetResult>>,
    },
    StartupCheck {
        response_tx: oneshot::Sender<Result<RouterStartupCheckReport>>,
    },
}

/// Handle for communicating with the router thread (Send+Sync)
#[derive(Clone)]
pub struct RouterHandle {
    tx: mpsc::Sender<RouterRequest>,
}

impl RouterHandle {
    /// Request a single-hop quote from the router thread.
    ///
    /// `is_sell_base = true` means base -> USDC quote via
    /// `pool::get_quote_quantity_out`.
    /// `is_sell_base = false` means USDC -> base quote via
    /// `pool::get_base_quantity_out`.
    pub async fn quote_single_hop(
        &self,
        pool_id: PoolId,
        input_amount: u64,
        is_sell_base: bool,
    ) -> Result<SingleHopQuote> {
        let (response_tx, response_rx) = oneshot::channel();

        self.tx
            .send(RouterRequest::SingleHop {
                pool_id,
                input_amount,
                is_sell_base,
                response_tx,
            })
            .map_err(|_| anyhow!("Router thread has shut down"))?;

        response_rx
            .await
            .map_err(|_| anyhow!("Router thread dropped response channel"))?
    }

    /// Request a two-hop quote from the router thread
    pub async fn quote_two_hop(
        &self,
        from_pool: PoolId,
        to_pool: PoolId,
        input_amount: u64,
    ) -> Result<TwoHopQuote> {
        let (response_tx, response_rx) = oneshot::channel();

        self.tx
            .send(RouterRequest::TwoHop {
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

    /// Execute a direct swap through MoveVM pool::swap_exact_*.
    pub async fn execute_single_hop_swap(
        &self,
        pool_id: PoolId,
        input_amount: u64,
        deep_amount: u64,
        is_sell_base: bool,
    ) -> Result<SingleHopSwapResult> {
        let (response_tx, response_rx) = oneshot::channel();

        self.tx
            .send(RouterRequest::ExecuteSingleHop {
                pool_id,
                input_amount,
                deep_amount,
                is_sell_base,
                response_tx,
            })
            .map_err(|_| anyhow!("Router thread has shut down"))?;

        response_rx
            .await
            .map_err(|_| anyhow!("Router thread dropped response channel"))?
    }

    /// Execute a two-hop swap through MoveVM (A -> USDC -> B).
    pub async fn execute_two_hop_swap(
        &self,
        from_pool: PoolId,
        to_pool: PoolId,
        input_amount: u64,
        deep_amount: u64,
    ) -> Result<TwoHopSwapResult> {
        let (response_tx, response_rx) = oneshot::channel();

        self.tx
            .send(RouterRequest::ExecuteTwoHop {
                from_pool,
                to_pool,
                input_amount,
                deep_amount,
                response_tx,
            })
            .map_err(|_| anyhow!("Router thread has shut down"))?;

        response_rx
            .await
            .map_err(|_| anyhow!("Router thread dropped response channel"))?
    }

    /// Ensure the debug pool (DBG/USDC) exists and is seeded in the VM.
    pub async fn ensure_debug_pool(&self) -> Result<DebugPoolInfo> {
        let (response_tx, response_rx) = oneshot::channel();
        self.tx
            .send(RouterRequest::EnsureDebugPool { response_tx })
            .map_err(|_| anyhow!("Router thread has shut down"))?;

        response_rx
            .await
            .map_err(|_| anyhow!("Router thread dropped response channel"))?
    }

    /// Ensure the debug pool exists with caller-provided config.
    ///
    /// If the debug pool already exists with different config, this returns an
    /// error because DeepBook allows only one pool per token pair in this VM
    /// runtime. Restart backend to reconfigure.
    pub async fn ensure_debug_pool_with_config(
        &self,
        config: DebugPoolCreateConfig,
    ) -> Result<DebugPoolInfo> {
        let (response_tx, response_rx) = oneshot::channel();
        self.tx
            .send(RouterRequest::EnsureDebugPoolWithConfig {
                config,
                response_tx,
            })
            .map_err(|_| anyhow!("Router thread has shut down"))?;

        response_rx
            .await
            .map_err(|_| anyhow!("Router thread dropped response channel"))?
    }

    /// Split and transfer a faucet coin via real MoveVM PTB execution.
    pub async fn vm_faucet(&self, coin_type: String, amount: u64) -> Result<VmFaucetResult> {
        let (response_tx, response_rx) = oneshot::channel();
        self.tx
            .send(RouterRequest::VmFaucet {
                coin_type,
                amount,
                response_tx,
            })
            .map_err(|_| anyhow!("Router thread has shut down"))?;

        response_rx
            .await
            .map_err(|_| anyhow!("Router thread dropped response channel"))?
    }

    /// Return the router startup self-check report.
    pub async fn startup_check(&self) -> Result<RouterStartupCheckReport> {
        let (response_tx, response_rx) = oneshot::channel();
        self.tx
            .send(RouterRequest::StartupCheck { response_tx })
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
/// 6. Executes a local-VM router health check
/// 7. Signals ready
/// 8. Loops processing quote requests
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
                match req {
                    RouterRequest::TwoHop {
                        from_pool,
                        to_pool,
                        input_amount,
                        response_tx,
                    } => {
                        let result =
                            execute_two_hop_quote(&mut env_state, from_pool, to_pool, input_amount);
                        let _ = response_tx.send(result);
                    }
                    RouterRequest::SingleHop {
                        pool_id,
                        input_amount,
                        is_sell_base,
                        response_tx,
                    } => {
                        let result = execute_single_hop_quote(
                            &mut env_state,
                            pool_id,
                            input_amount,
                            is_sell_base,
                        );
                        let _ = response_tx.send(result);
                    }
                    RouterRequest::ExecuteSingleHop {
                        pool_id,
                        input_amount,
                        deep_amount,
                        is_sell_base,
                        response_tx,
                    } => {
                        let result = execute_single_hop_swap(
                            &mut env_state,
                            pool_id,
                            input_amount,
                            deep_amount,
                            is_sell_base,
                        );
                        let _ = response_tx.send(result);
                    }
                    RouterRequest::ExecuteTwoHop {
                        from_pool,
                        to_pool,
                        input_amount,
                        deep_amount,
                        response_tx,
                    } => {
                        let result = execute_two_hop_swap(
                            &mut env_state,
                            from_pool,
                            to_pool,
                            input_amount,
                            deep_amount,
                        );
                        let _ = response_tx.send(result);
                    }
                    RouterRequest::EnsureDebugPool { response_tx } => {
                        let result = ensure_debug_pool(&mut env_state);
                        let _ = response_tx.send(result);
                    }
                    RouterRequest::EnsureDebugPoolWithConfig {
                        config,
                        response_tx,
                    } => {
                        let result = ensure_debug_pool_with_config(&mut env_state, config);
                        let _ = response_tx.send(result);
                    }
                    RouterRequest::VmFaucet {
                        coin_type,
                        amount,
                        response_tx,
                    } => {
                        let result = execute_vm_faucet(&mut env_state, &coin_type, amount);
                        let _ = response_tx.send(result);
                    }
                    RouterRequest::StartupCheck { response_tx } => {
                        let _ = response_tx.send(Ok(env_state.startup_check.clone()));
                    }
                }
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
    coin_reserve_cache: HashMap<String, AccountAddress>,
    debug_treasury_id: Option<AccountAddress>,
    router_deployed: bool,
    startup_check: RouterStartupCheckReport,
    next_clock_timestamp_ms: u64,
    debug_pool_config: DebugPoolCreateConfig,
    debug_pool_info: Option<DebugPoolInfo>,
}

#[derive(Debug, Clone)]
struct ReserveCoinCandidate {
    object_id: String,
    version: u64,
    type_string: String,
    bcs: Vec<u8>,
    value: u64,
}

struct PoolCacheEntry {
    pool_addr: AccountAddress,
    pool_type: TypeTag,
}

impl RouterEnvState {
    fn next_clock_input(&mut self) -> Result<ObjectInput> {
        let timestamp_ms = self.next_clock_timestamp_ms;
        self.next_clock_timestamp_ms = self
            .next_clock_timestamp_ms
            .saturating_add(SYNTHETIC_CLOCK_STEP_MS);
        build_clock_input(timestamp_ms)
    }

    fn clock_now_ms(&self) -> u64 {
        self.next_clock_timestamp_ms
    }
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

    // Debug pool creation needs DeepBook's shared Registry object.
    // Load it up front so ensure_debug_pool can run fully in local VM.
    load_grpc_object_into_env(
        &mut env,
        &rt,
        &grpc,
        COIN_REGISTRY_OBJECT_ID,
        "Sui Coin Registry",
    )?;
    load_grpc_object_into_env(
        &mut env,
        &rt,
        &grpc,
        DEEPBOOK_REGISTRY_ID,
        "DeepBook Registry",
    )?;
    load_registry_inner_dynamic_field(&mut env, &rt, &grpc)?;

    // Load all pool states
    let mut pool_cache = HashMap::new();
    let mut target_epoch: Option<u64> = None;
    for (pool_id, file_path) in pool_files {
        let path = Path::new(file_path);
        if !path.exists() {
            tracing::warn!(
                "Router: skipping {} - file not found: {}",
                pool_id.display_name(),
                file_path
            );
            continue;
        }

        let config = DeepBookConfig::for_pool(*pool_id);
        let pool_wrapper_id = config.pool_wrapper.clone();
        let mut loader = StateLoader::with_config(config);
        loader
            .load_from_file(path)
            .map_err(|e| anyhow!("Router: failed to load {}: {}", file_path, e))?;

        if let Some(pool_epoch) = extract_pool_epoch(&loader) {
            target_epoch = Some(target_epoch.map_or(pool_epoch, |current| current.max(pool_epoch)));
        }

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

        let synthesized_accounts =
            synthesize_account_dynamic_fields_for_router(&mut env, &mut bcs_converter, &loader)?;
        if synthesized_accounts > 0 {
            tracing::info!(
                "Router: synthesized {} state.accounts dynamic fields for {}",
                synthesized_accounts,
                pool_id.display_name()
            );
        }

        let synthesized_history =
            synthesize_history_volume_fields_for_router(&mut env, &mut bcs_converter, &loader)?;
        if synthesized_history > 0 {
            tracing::info!(
                "Router: synthesized {} history.historic_volumes fields for {}",
                synthesized_history,
                pool_id.display_name()
            );
        }

        // Cache pool entry for PTB construction
        if loader.get_object(&pool_wrapper_id).is_some() {
            let (base_type, quote_type) = match pool_id {
                PoolId::SuiUsdc => (SUI_TYPE, USDC_TYPE),
                PoolId::WalUsdc => (WAL_TYPE, USDC_TYPE),
                PoolId::DeepUsdc => (DEEP_TYPE, USDC_TYPE),
                PoolId::DebugUsdc => (DEBUG_TYPE, USDC_TYPE),
            };

            let pool_type = build_pool_type_tag(base_type, quote_type)?;
            let pool_addr = AccountAddress::from_hex_literal(&pool_wrapper_id)?;
            pool_cache.insert(
                *pool_id,
                PoolCacheEntry {
                    pool_addr,
                    pool_type,
                },
            );
        }

        tracing::info!("Router: loaded {} pool state", pool_id.display_name());
    }

    if let Some(epoch) = target_epoch {
        env.config_mut().epoch = epoch;
        tracing::info!("Router: set simulation epoch to {}", epoch);
    }

    // Create synthetic Clock object at 0x6
    create_clock_object(&mut env, SYNTHETIC_CLOCK_START_MS)?;

    // Compile and deploy router contract for two-hop quotes.
    deploy_router_contract(&mut env)?;

    let mut state = RouterEnvState {
        env,
        pool_cache,
        coin_reserve_cache: HashMap::new(),
        debug_treasury_id: None,
        router_deployed: true,
        startup_check: RouterStartupCheckReport::default(),
        next_clock_timestamp_ms: SYNTHETIC_CLOCK_START_MS,
        debug_pool_config: DebugPoolCreateConfig::default(),
        debug_pool_info: None,
    };

    bootstrap_mainnet_reserve_coins(&mut state, &rt, &grpc)?;

    // Explicit startup self-check. This must pass before backend starts.
    let report = run_startup_self_check(&mut state)?;
    state.startup_check = report;

    Ok(state)
}

fn load_grpc_object_into_env(
    env: &mut SimulationEnvironment,
    rt: &tokio::runtime::Runtime,
    grpc: &sui_transport::grpc::GrpcClient,
    object_id: &str,
    object_name: &str,
) -> Result<()> {
    let object_addr = AccountAddress::from_hex_literal(object_id)?;
    if env.get_object(&object_addr).is_some() {
        return Ok(());
    }

    let object = rt
        .block_on(grpc.get_object(object_id))?
        .ok_or_else(|| anyhow!("{} not found via gRPC: {}", object_name, object_id))?;

    let bcs_bytes = object
        .bcs
        .ok_or_else(|| anyhow!("{} missing BCS payload: {}", object_name, object_id))?;
    let type_string = object.type_string.clone();
    let owner = object.owner.clone();
    let is_shared = matches!(owner, GrpcOwner::Shared { .. });
    let is_immutable = matches!(owner, GrpcOwner::Immutable);
    let version = object.version;

    if let GrpcOwner::Object(parent_id_hex) = owner {
        let parent_id = AccountAddress::from_hex_literal(&parent_id_hex)?;
        let child_id = AccountAddress::from_hex_literal(object_id)?;
        let field_type = type_string
            .as_ref()
            .ok_or_else(|| anyhow!("{} missing type string: {}", object_name, object_id))?;
        let field_type_tag = SimulationEnvironment::parse_type_string(field_type)
            .ok_or_else(|| anyhow!("Failed to parse field type {}", field_type))?;
        env.set_dynamic_field(parent_id, child_id, field_type_tag, bcs_bytes);
    } else {
        env.load_object_from_data(
            object_id,
            bcs_bytes,
            type_string.as_deref(),
            is_shared,
            is_immutable,
            version,
        )?;
    }

    tracing::info!(
        "Router: loaded {} ({}, version={})",
        object_name,
        object_id,
        version
    );

    Ok(())
}

fn load_registry_inner_dynamic_field(
    env: &mut SimulationEnvironment,
    rt: &tokio::runtime::Runtime,
    grpc: &sui_transport::grpc::GrpcClient,
) -> Result<()> {
    let registry_addr = AccountAddress::from_hex_literal(DEEPBOOK_REGISTRY_ID)?;
    let registry_obj = env
        .get_object(&registry_addr)
        .ok_or_else(|| anyhow!("Registry object missing in env: {}", registry_addr))?;

    if registry_obj.bcs_bytes.len() < 72 {
        return Err(anyhow!(
            "Registry object BCS too short ({}), expected at least 72 bytes",
            registry_obj.bcs_bytes.len()
        ));
    }

    let mut inner_id_bytes = [0u8; AccountAddress::LENGTH];
    inner_id_bytes.copy_from_slice(&registry_obj.bcs_bytes[32..64]);
    let inner_id = AccountAddress::new(inner_id_bytes);

    let mut version_bytes = [0u8; 8];
    version_bytes.copy_from_slice(&registry_obj.bcs_bytes[64..72]);
    let current_version = u64::from_le_bytes(version_bytes);

    let key_bytes = bcs::to_bytes(&current_version)?;
    let child_id = derive_dynamic_field_id(inner_id, &TypeTag::U64, &key_bytes)
        .map_err(|e| anyhow!("Failed to derive registry inner dynamic field id: {}", e))?;
    let child_id_hex = child_id.to_hex_literal();

    load_grpc_object_into_env(
        env,
        rt,
        grpc,
        &child_id_hex,
        "DeepBook RegistryInner dynamic field",
    )?;

    Ok(())
}

fn coin_object_type(coin_type: &str) -> String {
    format!("0x2::coin::Coin<{}>", coin_type)
}

fn normalize_type_string(type_string: &str) -> String {
    type_string.replace(' ', "")
}

fn parse_coin_value_from_bcs(bcs: &[u8]) -> Option<u64> {
    if bcs.len() < 40 {
        return None;
    }
    let mut bytes = [0u8; 8];
    bytes.copy_from_slice(&bcs[32..40]);
    Some(u64::from_le_bytes(bytes))
}

fn find_reserve_candidate(
    object: GrpcObject,
    expected_coin_object_tag: &TypeTag,
) -> Option<ReserveCoinCandidate> {
    let bcs = object.bcs?;
    let type_string = object.type_string?;
    let observed_tag = TypeTag::from_str(&type_string).ok()?;
    if &observed_tag != expected_coin_object_tag {
        return None;
    }
    if !matches!(object.owner, GrpcOwner::Address(_)) {
        return None;
    }
    let value = parse_coin_value_from_bcs(&bcs)?;
    Some(ReserveCoinCandidate {
        object_id: object.object_id,
        version: object.version,
        type_string,
        bcs,
        value,
    })
}

fn bootstrap_mainnet_reserve_coins(
    state: &mut RouterEnvState,
    rt: &tokio::runtime::Runtime,
    grpc: &sui_transport::grpc::GrpcClient,
) -> Result<()> {
    let reserve_types = [SUI_TYPE, USDC_TYPE, WAL_TYPE, DEEP_TYPE];
    let mut candidates: HashMap<&'static str, ReserveCoinCandidate> = HashMap::new();
    let expected_types: HashMap<&'static str, TypeTag> = reserve_types
        .iter()
        .map(|coin_type| {
            let coin_obj = coin_object_type(coin_type);
            let tag = TypeTag::from_str(&coin_obj)
                .map_err(|e| anyhow!("Invalid reserve coin type tag {}: {}", coin_obj, e))?;
            Ok((*coin_type, tag))
        })
        .collect::<Result<HashMap<_, _>>>()?;

    let service_info = rt.block_on(grpc.get_service_info())?;
    let latest = service_info.checkpoint_height;
    let start = latest.saturating_sub(MAINNET_RESERVE_SCAN_WINDOW);

    tracing::info!(
        "Router: bootstrapping VM reserve coins from checkpoints {}..={} (latest={})",
        start,
        latest,
        latest
    );

    for checkpoint in (start..=latest).rev() {
        let cp_opt = match rt.block_on(grpc.get_checkpoint(checkpoint)) {
            Ok(cp) => cp,
            Err(e) => {
                tracing::warn!(
                    "Router: skipping checkpoint {} during reserve bootstrap: {}",
                    checkpoint,
                    e
                );
                continue;
            }
        };

        let Some(cp) = cp_opt else {
            continue;
        };

        for object in cp.objects {
            for coin_type in reserve_types {
                let Some(expected) = expected_types.get(coin_type) else {
                    continue;
                };
                let Some(candidate) = find_reserve_candidate(object.clone(), expected) else {
                    continue;
                };
                let replace = candidates
                    .get(coin_type)
                    .map(|existing| candidate.value > existing.value)
                    .unwrap_or(true);
                if replace {
                    candidates.insert(coin_type, candidate);
                }
            }
        }
    }

    let missing: Vec<&str> = reserve_types
        .iter()
        .copied()
        .filter(|coin_type| !candidates.contains_key(coin_type))
        .collect();
    if !missing.is_empty() {
        return Err(anyhow!(
            "Router reserve bootstrap failed: missing checkpoint coin objects for [{}] in the last {} checkpoints",
            missing.join(", "),
            MAINNET_RESERVE_SCAN_WINDOW
        ));
    }

    for coin_type in reserve_types {
        let candidate = candidates
            .remove(coin_type)
            .ok_or_else(|| anyhow!("Missing reserve candidate for {}", coin_type))?;
        let reserve_id = AccountAddress::from_hex_literal(&candidate.object_id)?;
        if state.env.get_object(&reserve_id).is_none() {
            state.env.load_object_from_data(
                &candidate.object_id,
                candidate.bcs.clone(),
                Some(&candidate.type_string),
                false,
                false,
                candidate.version,
            )?;
        }
        state
            .coin_reserve_cache
            .insert(coin_type.to_string(), reserve_id);
        tracing::info!(
            "Router: checkpoint-backed reserve loaded for {} at {} (value={}, version={})",
            coin_type,
            reserve_id,
            candidate.value,
            candidate.version
        );
    }

    Ok(())
}

fn pool_types(pool_id: PoolId) -> (&'static str, &'static str) {
    match pool_id {
        PoolId::SuiUsdc => (SUI_TYPE, USDC_TYPE),
        PoolId::WalUsdc => (WAL_TYPE, USDC_TYPE),
        PoolId::DeepUsdc => (DEEP_TYPE, USDC_TYPE),
        PoolId::DebugUsdc => (DEBUG_TYPE, USDC_TYPE),
    }
}

fn sync_dynamic_field_entries(
    state: &mut RouterEnvState,
    effects: &sui_sandbox_core::ptb::TransactionEffects,
) {
    let mut object_bytes_synced = 0usize;
    for (object_id, bytes) in &effects.created_object_bytes {
        if state.env.get_object(object_id).is_some()
            && state.env.set_object_bytes(*object_id, bytes.clone()).is_ok()
        {
            object_bytes_synced += 1;
        }
    }
    for (object_id, bytes) in &effects.mutated_object_bytes {
        if state.env.get_object(object_id).is_some()
            && state.env.set_object_bytes(*object_id, bytes.clone()).is_ok()
        {
            object_bytes_synced += 1;
        }
    }

    for ((parent_id, child_id), (type_tag, bytes)) in &effects.dynamic_field_entries {
        let corrected_type_tag = normalize_dynamic_field_type_tag(type_tag);
        state
            .env
            .set_dynamic_field(*parent_id, *child_id, corrected_type_tag, bytes.clone());
        if state.env.get_object(child_id).is_some()
            && state.env.set_object_bytes(*child_id, bytes.clone()).is_ok()
        {
            object_bytes_synced += 1;
        }
    }

    // Some sandbox builds do not fully mirror dynamic field updates in
    // `dynamic_field_entries`, but the created/mutated field objects still appear
    // in object_changes with Owner::Object(parent). Backfill those entries.
    let mut backfilled = 0usize;
    for change in &effects.object_changes {
        match change {
            sui_sandbox_core::ptb::ObjectChange::Created {
                id,
                owner,
                object_type: Some(type_tag),
            } => {
                if !type_tag.to_string().contains("::dynamic_field::Field<") {
                    continue;
                }
                let Some(parent_id) = parse_parent_from_owner_debug(owner) else {
                    continue;
                };
                if let Some(bytes) = effects.created_object_bytes.get(id) {
                    let corrected_type_tag = normalize_dynamic_field_type_tag(type_tag);
                    state
                        .env
                        .set_dynamic_field(parent_id, *id, corrected_type_tag, bytes.clone());
                    if state.env.get_object(id).is_some()
                        && state.env.set_object_bytes(*id, bytes.clone()).is_ok()
                    {
                        object_bytes_synced += 1;
                    }
                    backfilled += 1;
                }
            }
            sui_sandbox_core::ptb::ObjectChange::Mutated {
                id,
                owner,
                object_type: Some(type_tag),
            } => {
                if !type_tag.to_string().contains("::dynamic_field::Field<") {
                    continue;
                }
                let Some(parent_id) = parse_parent_from_owner_debug(owner) else {
                    continue;
                };
                if let Some(bytes) = effects.mutated_object_bytes.get(id) {
                    let corrected_type_tag = normalize_dynamic_field_type_tag(type_tag);
                    state
                        .env
                        .set_dynamic_field(parent_id, *id, corrected_type_tag, bytes.clone());
                    if state.env.get_object(id).is_some()
                        && state.env.set_object_bytes(*id, bytes.clone()).is_ok()
                    {
                        object_bytes_synced += 1;
                    }
                    backfilled += 1;
                }
            }
            _ => {}
        }
    }

    let mut reconciled = 0usize;
    let pool_ids: Vec<PoolId> = state.pool_cache.keys().copied().collect();
    for pool_id in pool_ids {
        match reconcile_pool_inner_version_from_dynamic_fields(state, pool_id) {
            Ok(true) => reconciled += 1,
            Ok(false) => {}
            Err(e) => tracing::warn!(
                "Router: failed to reconcile {} pool wrapper version: {}",
                pool_id.display_name(),
                e
            ),
        }
    }

    // Work around a sandbox gap: mutated dynamic-field child objects may be present
    // in `mutated_object_bytes` without an updated entry in `dynamic_field_entries`.
    // Refresh PoolInner children explicitly so order-book mutations persist across PTBs.
    let mut refreshed = 0usize;
    for pool_entry in state.pool_cache.values() {
        let Some(pool_obj) = state.env.get_object(&pool_entry.pool_addr) else {
            continue;
        };
        if pool_obj.bcs_bytes.len() < 72 {
            continue;
        }

        let mut inner_parent_bytes = [0u8; AccountAddress::LENGTH];
        inner_parent_bytes.copy_from_slice(&pool_obj.bcs_bytes[32..64]);
        let inner_parent = AccountAddress::new(inner_parent_bytes);

        let mut version_bytes = [0u8; 8];
        version_bytes.copy_from_slice(&pool_obj.bcs_bytes[64..72]);
        let inner_version = u64::from_le_bytes(version_bytes);

        let Ok(key_bytes) = bcs::to_bytes(&inner_version) else {
            continue;
        };
        let Ok(inner_child) = derive_dynamic_field_id(inner_parent, &TypeTag::U64, &key_bytes)
        else {
            continue;
        };

        let Some(mutated_bytes) = effects.mutated_object_bytes.get(&inner_child) else {
            continue;
        };
        let Some((type_tag, _existing_bytes)) = state
            .env
            .get_dynamic_field(inner_parent, inner_child)
            .cloned()
        else {
            continue;
        };

        state
            .env
            .set_dynamic_field(inner_parent, inner_child, type_tag, mutated_bytes.clone());
        refreshed += 1;
    }

    if refreshed > 0 {
        tracing::info!(
            "Router: refreshed {} PoolInner dynamic-field children from mutated_object_bytes",
            refreshed
        );
    }
    if object_bytes_synced > 0 {
        tracing::info!(
            "Router: synchronized {} object byte snapshots from PTB effects",
            object_bytes_synced
        );
    }
    if reconciled > 0 {
        tracing::info!(
            "Router: reconciled {} pool wrapper inner versions from dynamic fields",
            reconciled
        );
    }
    if backfilled > 0 {
        tracing::info!(
            "Router: backfilled {} dynamic fields from object_changes",
            backfilled
        );
    }
}

fn normalize_dynamic_field_type_tag(type_tag: &TypeTag) -> TypeTag {
    let type_str = type_tag.to_string();
    if !type_str.contains("::dynamic_field::Field<u64, vector<") || !type_str.contains(DEEPBOOK_PACKAGE) {
        return type_tag.clone();
    }

    let Some(vector_start) = type_str.find("vector<") else {
        return type_tag.clone();
    };
    let element_start = vector_start + "vector<".len();
    let remaining = &type_str[element_start..];

    let mut depth = 1usize;
    let mut element_end = None;
    for (idx, ch) in remaining.char_indices() {
        match ch {
            '<' => depth += 1,
            '>' => {
                depth -= 1;
                if depth == 0 {
                    element_end = Some(idx);
                    break;
                }
            }
            _ => {}
        }
    }
    let Some(element_end) = element_end else {
        return type_tag.clone();
    };

    let element_type = &remaining[..element_end];
    let prefix = &type_str[..vector_start];
    let suffix = &type_str[element_start + element_end + 1..];
    let corrected = format!(
        "{}{}::big_vector::Slice<{}>{}",
        prefix, DEEPBOOK_PACKAGE, element_type, suffix
    );

    TypeTag::from_str(&corrected).unwrap_or_else(|_| type_tag.clone())
}

fn parse_parent_from_owner_debug(owner: &impl std::fmt::Debug) -> Option<AccountAddress> {
    let owner_debug = format!("{:?}", owner);
    if let Some(object_owner) = owner_debug
        .strip_prefix("Object(")
        .and_then(|raw| raw.strip_suffix(')'))
        .map(str::trim)
    {
        let normalized = if object_owner.starts_with("0x") {
            object_owner.to_string()
        } else {
            format!("0x{}", object_owner)
        };
        if let Ok(addr) = AccountAddress::from_hex_literal(&normalized) {
            return Some(addr);
        }
    }

    // Fallback: parse the first `0x...` token from debug output.
    let start = owner_debug.find("0x")?;
    let hex_tail = &owner_debug[start + 2..];
    let hex_len = hex_tail
        .chars()
        .take_while(|c| c.is_ascii_hexdigit())
        .count();
    if hex_len == 0 {
        return None;
    }

    let candidate = format!("0x{}", &hex_tail[..hex_len]);
    AccountAddress::from_hex_literal(&candidate).ok()
}

fn parse_dynamic_field_u64_name(field_bytes: &[u8]) -> Option<u64> {
    // Field<K, V> BCS layout starts with UID (32 bytes) followed by `name: K`.
    if field_bytes.len() < 40 {
        return None;
    }

    let mut key_bytes = [0u8; 8];
    key_bytes.copy_from_slice(&field_bytes[32..40]);
    Some(u64::from_le_bytes(key_bytes))
}

fn patch_pool_big_vector_header_from_created_slice(
    state: &mut RouterEnvState,
    pool_id: PoolId,
    big_vector_parent: AccountAddress,
    slice_key: u64,
) -> Result<bool> {
    let pool_addr = match state.pool_cache.get(&pool_id) {
        Some(entry) => entry.pool_addr,
        None => return Ok(false),
    };
    let pool_obj = match state.env.get_object(&pool_addr) {
        Some(obj) => obj,
        None => return Ok(false),
    };
    if pool_obj.bcs_bytes.len() < 72 {
        return Ok(false);
    }

    let mut inner_parent_bytes = [0u8; AccountAddress::LENGTH];
    inner_parent_bytes.copy_from_slice(&pool_obj.bcs_bytes[32..64]);
    let inner_parent = AccountAddress::new(inner_parent_bytes);
    let mut version_bytes = [0u8; 8];
    version_bytes.copy_from_slice(&pool_obj.bcs_bytes[64..72]);
    let inner_version = u64::from_le_bytes(version_bytes);
    let key_bytes = bcs::to_bytes(&inner_version)?;
    let inner_child = derive_dynamic_field_id(inner_parent, &TypeTag::U64, &key_bytes)?;

    let Some((field_type, field_bytes)) = state.env.get_dynamic_field(inner_parent, inner_child).cloned()
    else {
        return Ok(false);
    };
    if field_bytes.len() < 40 {
        return Ok(false);
    }

    let mut patched_field_bytes = field_bytes.clone();
    let value_bytes = &mut patched_field_bytes[40..];
    let parent_raw = big_vector_parent.as_ref();
    let mut patched = false;
    let mut idx = 0usize;
    while idx + AccountAddress::LENGTH <= value_bytes.len() {
        if &value_bytes[idx..idx + AccountAddress::LENGTH] != parent_raw {
            idx += 1;
            continue;
        }
        // BigVector layout:
        // id (32), depth (1), length (8), max_slice_size (8), max_fan_out (8), root_id (8), last_id (8)
        if idx + 73 > value_bytes.len() {
            break;
        }
        let length_off = idx + 33;
        let root_id_off = idx + 57;
        let last_id_off = idx + 65;

        let mut length_bytes = [0u8; 8];
        length_bytes.copy_from_slice(&value_bytes[length_off..length_off + 8]);
        let current_length = u64::from_le_bytes(length_bytes);

        let mut root_bytes = [0u8; 8];
        root_bytes.copy_from_slice(&value_bytes[root_id_off..root_id_off + 8]);
        let current_root = u64::from_le_bytes(root_bytes);

        let mut last_bytes = [0u8; 8];
        last_bytes.copy_from_slice(&value_bytes[last_id_off..last_id_off + 8]);
        let current_last = u64::from_le_bytes(last_bytes);

        let new_length = current_length.max(1);
        let new_root = if current_root == 0 {
            slice_key
        } else {
            current_root
        };
        let new_last = current_last.max(slice_key);

        value_bytes[length_off..length_off + 8].copy_from_slice(&new_length.to_le_bytes());
        value_bytes[root_id_off..root_id_off + 8].copy_from_slice(&new_root.to_le_bytes());
        value_bytes[last_id_off..last_id_off + 8].copy_from_slice(&new_last.to_le_bytes());

        state
            .env
            .set_dynamic_field(inner_parent, inner_child, field_type.clone(), patched_field_bytes.clone());
        if state.env.get_object(&inner_child).is_some() {
            state
                .env
                .set_object_bytes(inner_child, patched_field_bytes.clone())
                .map_err(|e| anyhow!("failed patching PoolInner bytes {}: {}", inner_child, e))?;
        }

        tracing::info!(
            "Router: patched {} BigVector header parent={} key={} length {}->{} root {}->{} last {}->{}",
            pool_id.display_name(),
            big_vector_parent,
            slice_key,
            current_length,
            new_length,
            current_root,
            new_root,
            current_last,
            new_last
        );
        patched = true;
        break;
    }

    Ok(patched)
}

fn scaled_mul_floor(lhs: u64, rhs: u64) -> u64 {
    ((lhs as u128 * rhs as u128) / 1_000_000_000u128) as u64
}

fn patch_pool_vault_tail_for_seed(
    state: &mut RouterEnvState,
    pool_id: PoolId,
    add_base: u64,
    add_quote: u64,
    add_deep: u64,
) -> Result<bool> {
    if add_base == 0 && add_quote == 0 && add_deep == 0 {
        return Ok(false);
    }

    let pool_addr = match state.pool_cache.get(&pool_id) {
        Some(entry) => entry.pool_addr,
        None => return Ok(false),
    };
    let pool_obj = match state.env.get_object(&pool_addr) {
        Some(obj) => obj,
        None => return Ok(false),
    };
    if pool_obj.bcs_bytes.len() < 72 {
        return Ok(false);
    }

    let mut inner_parent_bytes = [0u8; AccountAddress::LENGTH];
    inner_parent_bytes.copy_from_slice(&pool_obj.bcs_bytes[32..64]);
    let inner_parent = AccountAddress::new(inner_parent_bytes);
    let mut version_bytes = [0u8; 8];
    version_bytes.copy_from_slice(&pool_obj.bcs_bytes[64..72]);
    let inner_version = u64::from_le_bytes(version_bytes);
    let key_bytes = bcs::to_bytes(&inner_version)?;
    let inner_child = derive_dynamic_field_id(inner_parent, &TypeTag::U64, &key_bytes)?;

    let Some((field_type, field_bytes)) = state.env.get_dynamic_field(inner_parent, inner_child).cloned()
    else {
        return Ok(false);
    };
    if field_bytes.len() < 40 + 43 {
        return Ok(false);
    }

    let mut patched_field_bytes = field_bytes.clone();
    let value_bytes = &mut patched_field_bytes[40..];
    let value_len = value_bytes.len();
    let vault_start = value_len - 43;
    let deep_price_base_vec_len_off = value_len - 19;
    let deep_price_quote_vec_len_off = value_len - 10;
    let registered_pool_off = value_len - 1;

    // This tail layout assumption matches an empty DeepPrice:
    // [vault base/quote/deep (24)] [vec_len=0][cum_base=0][vec_len=0][cum_quote=0][registered_pool]
    if value_bytes[deep_price_base_vec_len_off] != 0
        || value_bytes[deep_price_quote_vec_len_off] != 0
        || value_bytes[registered_pool_off] > 1
    {
        return Ok(false);
    }

    let read_u64 = |buf: &[u8], off: usize| -> u64 {
        let mut bytes = [0u8; 8];
        bytes.copy_from_slice(&buf[off..off + 8]);
        u64::from_le_bytes(bytes)
    };

    let base_off = vault_start;
    let quote_off = vault_start + 8;
    let deep_off = vault_start + 16;
    let old_base = read_u64(value_bytes, base_off);
    let old_quote = read_u64(value_bytes, quote_off);
    let old_deep = read_u64(value_bytes, deep_off);

    let new_base = old_base.saturating_add(add_base);
    let new_quote = old_quote.saturating_add(add_quote);
    let new_deep = old_deep.saturating_add(add_deep);

    value_bytes[base_off..base_off + 8].copy_from_slice(&new_base.to_le_bytes());
    value_bytes[quote_off..quote_off + 8].copy_from_slice(&new_quote.to_le_bytes());
    value_bytes[deep_off..deep_off + 8].copy_from_slice(&new_deep.to_le_bytes());

    state
        .env
        .set_dynamic_field(inner_parent, inner_child, field_type.clone(), patched_field_bytes.clone());
    if state.env.get_object(&inner_child).is_some() {
        state
            .env
            .set_object_bytes(inner_child, patched_field_bytes.clone())
            .map_err(|e| anyhow!("failed patching PoolInner vault bytes {}: {}", inner_child, e))?;
    }

    tracing::info!(
        "Router: patched {} vault tail base {}->{} quote {}->{} deep {}->{}",
        pool_id.display_name(),
        old_base,
        new_base,
        old_quote,
        new_quote,
        old_deep,
        new_deep
    );
    Ok(true)
}

fn reconcile_pool_inner_version_from_dynamic_fields(
    state: &mut RouterEnvState,
    pool_id: PoolId,
) -> Result<bool> {
    let pool_addr = match state.pool_cache.get(&pool_id) {
        Some(entry) => entry.pool_addr,
        None => return Ok(false),
    };
    let pool_obj = match state.env.get_object(&pool_addr) {
        Some(obj) => obj,
        None => return Ok(false),
    };

    if pool_obj.bcs_bytes.len() < 72 {
        return Ok(false);
    }

    let mut parent_bytes = [0u8; AccountAddress::LENGTH];
    parent_bytes.copy_from_slice(&pool_obj.bcs_bytes[32..64]);
    let inner_parent = AccountAddress::new(parent_bytes);

    let mut current_version_bytes = [0u8; 8];
    current_version_bytes.copy_from_slice(&pool_obj.bcs_bytes[64..72]);
    let current_version = u64::from_le_bytes(current_version_bytes);

    let (base_type, quote_type) = pool_types(pool_id);
    let expected_inner = format!("::pool::PoolInner<{},{}>", base_type, quote_type);

    let mut latest_version = None::<u64>;
    for (_child_id, type_tag, bytes) in state.env.get_dynamic_fields_for_parent(inner_parent) {
        let type_str = type_tag.to_string().replace(' ', "");
        if !type_str.contains("::dynamic_field::Field<u64,") {
            continue;
        }
        if !type_str.contains(&expected_inner) {
            continue;
        }
        let Some(version_key) = parse_dynamic_field_u64_name(bytes) else {
            continue;
        };
        latest_version = Some(latest_version.map_or(version_key, |v| v.max(version_key)));
    }

    let Some(latest_version) = latest_version else {
        return Ok(false);
    };
    if latest_version <= current_version {
        return Ok(false);
    }

    let mut patched = pool_obj.bcs_bytes.clone();
    patched[64..72].copy_from_slice(&latest_version.to_le_bytes());
    state
        .env
        .set_object_bytes(pool_addr, patched)
        .map_err(|e| anyhow!("failed updating pool wrapper bytes for {}: {}", pool_addr, e))?;

    tracing::info!(
        "Router: patched {} wrapper inner.version {} -> {}",
        pool_id.display_name(),
        current_version,
        latest_version
    );
    Ok(true)
}

fn build_clock_input(timestamp_ms: u64) -> Result<ObjectInput> {
    let clock_addr = AccountAddress::from_hex_literal(CLOCK_OBJECT_ID)?;
    let mut clock_bytes = Vec::new();
    clock_bytes.extend_from_slice(clock_addr.as_ref());
    clock_bytes.extend_from_slice(&timestamp_ms.to_le_bytes());

    Ok(ObjectInput::Shared {
        id: clock_addr,
        bytes: clock_bytes,
        type_tag: Some(TypeTag::from_str("0x2::clock::Clock")?),
        version: Some(1),
        mutable: false,
    })
}

fn parse_u64_return(return_values: &[Vec<u8>], idx: usize, field_name: &str) -> Result<u64> {
    let bytes = return_values
        .get(idx)
        .ok_or_else(|| anyhow!("Missing {} return value", field_name))?;

    if bytes.len() < 8 {
        return Err(anyhow!(
            "Invalid {} bytes length: {}",
            field_name,
            bytes.len()
        ));
    }

    let mut value_bytes = [0u8; 8];
    value_bytes.copy_from_slice(&bytes[..8]);
    Ok(u64::from_le_bytes(value_bytes))
}

fn parse_u128_return(return_values: &[Vec<u8>], idx: usize, field_name: &str) -> Result<u128> {
    let bytes = return_values
        .get(idx)
        .ok_or_else(|| anyhow!("Missing {} return value", field_name))?;

    if bytes.len() < 16 {
        return Err(anyhow!(
            "Invalid {} bytes length: {}",
            field_name,
            bytes.len()
        ));
    }

    let mut value_bytes = [0u8; 16];
    value_bytes.copy_from_slice(&bytes[..16]);
    Ok(u128::from_le_bytes(value_bytes))
}

fn parse_u8_return(return_values: &[Vec<u8>], idx: usize, field_name: &str) -> Result<u8> {
    let bytes = return_values
        .get(idx)
        .ok_or_else(|| anyhow!("Missing {} return value", field_name))?;
    let value = bytes
        .first()
        .copied()
        .ok_or_else(|| anyhow!("Invalid {} bytes length: {}", field_name, bytes.len()))?;
    Ok(value)
}

fn parse_bool_return(return_values: &[Vec<u8>], idx: usize, field_name: &str) -> Result<bool> {
    Ok(parse_u8_return(return_values, idx, field_name)? != 0)
}

fn parse_u64_command_return(
    effects: &sui_sandbox_core::ptb::TransactionEffects,
    command_idx: usize,
    value_idx: usize,
    field_name: &str,
) -> Result<u64> {
    let command_returns = effects
        .return_values
        .get(command_idx)
        .ok_or_else(|| anyhow!("Missing return values for command {}", command_idx))?;
    parse_u64_return(command_returns, value_idx, field_name)
}

fn parse_u8_command_return(
    effects: &sui_sandbox_core::ptb::TransactionEffects,
    command_idx: usize,
    value_idx: usize,
    field_name: &str,
) -> Result<u8> {
    let command_returns = effects
        .return_values
        .get(command_idx)
        .ok_or_else(|| anyhow!("Missing return values for command {}", command_idx))?;
    parse_u8_return(command_returns, value_idx, field_name)
}

fn parse_u128_command_return(
    effects: &sui_sandbox_core::ptb::TransactionEffects,
    command_idx: usize,
    value_idx: usize,
    field_name: &str,
) -> Result<u128> {
    let command_returns = effects
        .return_values
        .get(command_idx)
        .ok_or_else(|| anyhow!("Missing return values for command {}", command_idx))?;
    parse_u128_return(command_returns, value_idx, field_name)
}

fn parse_bool_command_return(
    effects: &sui_sandbox_core::ptb::TransactionEffects,
    command_idx: usize,
    value_idx: usize,
    field_name: &str,
) -> Result<bool> {
    let command_returns = effects
        .return_values
        .get(command_idx)
        .ok_or_else(|| anyhow!("Missing return values for command {}", command_idx))?;
    parse_bool_return(command_returns, value_idx, field_name)
}

fn parse_vec_u64_command_return(
    effects: &sui_sandbox_core::ptb::TransactionEffects,
    command_idx: usize,
    value_idx: usize,
    field_name: &str,
) -> Result<Vec<u64>> {
    let command_returns = effects
        .return_values
        .get(command_idx)
        .ok_or_else(|| anyhow!("Missing return values for command {}", command_idx))?;
    let bytes = command_returns
        .get(value_idx)
        .ok_or_else(|| anyhow!("Missing {} return value", field_name))?;
    bcs::from_bytes::<Vec<u64>>(bytes)
        .map_err(|e| anyhow!("Failed to decode {} return value as vector<u64>: {}", field_name, e))
}

fn pool_shared_input(
    state: &RouterEnvState,
    pool_id: PoolId,
    mutable: bool,
) -> Result<ObjectInput> {
    let pool_entry = state
        .pool_cache
        .get(&pool_id)
        .ok_or_else(|| anyhow!("Pool {} not loaded in router", pool_id.display_name()))?;
    let pool_obj = state
        .env
        .get_object(&pool_entry.pool_addr)
        .ok_or_else(|| anyhow!("Pool object missing in env: {}", pool_entry.pool_addr))?;

    Ok(ObjectInput::Shared {
        id: pool_entry.pool_addr,
        bytes: pool_obj.bcs_bytes.clone(),
        type_tag: Some(pool_entry.pool_type.clone()),
        version: Some(pool_obj.version),
        mutable,
    })
}

fn registry_shared_input(state: &RouterEnvState, mutable: bool) -> Result<ObjectInput> {
    let registry_addr = AccountAddress::from_hex_literal(DEEPBOOK_REGISTRY_ID)?;
    let registry_obj = state
        .env
        .get_object(&registry_addr)
        .ok_or_else(|| anyhow!("Registry object missing in env: {}", registry_addr))?;

    Ok(ObjectInput::Shared {
        id: registry_addr,
        bytes: registry_obj.bcs_bytes.clone(),
        type_tag: Some(TypeTag::from_str(&format!(
            "{}::registry::Registry",
            DEEPBOOK_PACKAGE
        ))?),
        version: Some(registry_obj.version),
        mutable,
    })
}

fn coin_registry_shared_input(state: &RouterEnvState, mutable: bool) -> Result<ObjectInput> {
    let registry_addr = AccountAddress::from_hex_literal(COIN_REGISTRY_OBJECT_ID)?;
    let registry_obj = state
        .env
        .get_object(&registry_addr)
        .ok_or_else(|| anyhow!("Coin registry object missing in env: {}", registry_addr))?;

    Ok(ObjectInput::Shared {
        id: registry_addr,
        bytes: registry_obj.bcs_bytes.clone(),
        type_tag: Some(TypeTag::from_str("0x2::coin_registry::CoinRegistry")?),
        version: Some(registry_obj.version),
        mutable,
    })
}

fn admin_cap_input(state: &RouterEnvState) -> Result<ObjectInput> {
    let admin_cap_addr = AccountAddress::from_hex_literal(DEBUG_ADMIN_CAP_ID)?;
    let admin_cap_obj = state.env.get_object(&admin_cap_addr).ok_or_else(|| {
        anyhow!(
            "DeepBook admin cap object missing in env: {}",
            admin_cap_addr
        )
    })?;

    Ok(ObjectInput::ImmRef {
        id: admin_cap_addr,
        bytes: admin_cap_obj.bcs_bytes.clone(),
        type_tag: Some(TypeTag::from_str(&format!(
            "{}::registry::DeepbookAdminCap",
            DEEPBOOK_PACKAGE
        ))?),
        version: Some(admin_cap_obj.version),
    })
}

fn ensure_debug_admin_cap(state: &mut RouterEnvState) -> Result<()> {
    let admin_cap_addr = AccountAddress::from_hex_literal(DEBUG_ADMIN_CAP_ID)?;
    if state.env.get_object(&admin_cap_addr).is_some() {
        return Ok(());
    }

    // DeepbookAdminCap has a single UID field, encoded as its object id bytes.
    let mut bcs_bytes = Vec::with_capacity(AccountAddress::LENGTH);
    bcs_bytes.extend_from_slice(admin_cap_addr.as_ref());

    state.env.load_object_from_data(
        DEBUG_ADMIN_CAP_ID,
        bcs_bytes,
        Some(&format!("{}::registry::DeepbookAdminCap", DEEPBOOK_PACKAGE)),
        false,
        false,
        1,
    )?;

    tracing::info!(
        "Router: synthesized DeepBook admin cap for debug pool creation ({})",
        DEBUG_ADMIN_CAP_ID
    );
    Ok(())
}

fn find_created_object_id_by_type(
    effects: &sui_sandbox_core::ptb::TransactionEffects,
    expected_type: &str,
) -> Option<AccountAddress> {
    let expected_normalized = normalize_type_string(expected_type);
    effects.object_changes.iter().find_map(|change| match change {
        sui_sandbox_core::ptb::ObjectChange::Created {
            id,
            object_type: Some(type_tag),
            ..
        } => {
            let observed = normalize_type_string(&type_tag.to_string());
            (observed == expected_normalized).then_some(*id)
        }
        _ => None,
    })
}

fn ensure_debug_treasury(state: &mut RouterEnvState) -> Result<AccountAddress> {
    if let Some(existing) = state.debug_treasury_id {
        if state.env.get_object(&existing).is_some() {
            return Ok(existing);
        }
        state.debug_treasury_id = None;
    }

    let treasury_tag = TypeTag::from_str(DEBUG_TREASURY_TYPE)?;
    if let Some(existing) = state
        .env
        .list_objects()
        .into_iter()
        .find(|obj| obj.type_tag == treasury_tag)
        .map(|obj| obj.id)
    {
        state.debug_treasury_id = Some(existing);
        return Ok(existing);
    }

    let token_cfg = state.debug_pool_config.clone();
    let router_addr = AccountAddress::from_hex_literal(ROUTER_PACKAGE_ADDR)?;
    let result = state.env.execute_ptb(
        vec![
            InputValue::Object(coin_registry_shared_input(state, true)?),
            InputValue::Pure(bcs::to_bytes(&token_cfg.token_decimals)?),
            InputValue::Pure(bcs::to_bytes(&token_cfg.token_symbol.as_bytes().to_vec())?),
            InputValue::Pure(bcs::to_bytes(&token_cfg.token_name.as_bytes().to_vec())?),
            InputValue::Pure(bcs::to_bytes(
                &token_cfg.token_description.as_bytes().to_vec(),
            )?),
            InputValue::Pure(bcs::to_bytes(&token_cfg.token_icon_url.as_bytes().to_vec())?),
        ],
        vec![Command::MoveCall {
            package: router_addr,
            module: Identifier::new("debug_token")?,
            function: Identifier::new("init_for_router")?,
            type_args: vec![],
            args: vec![
                Argument::Input(0),
                Argument::Input(1),
                Argument::Input(2),
                Argument::Input(3),
                Argument::Input(4),
                Argument::Input(5),
            ],
        }],
    );

    if !result.success {
        return Err(anyhow!(
            "debug treasury init failed: {}",
            result
                .raw_error
                .unwrap_or_else(|| "Unknown error".to_string())
        ));
    }
    let effects = result
        .effects
        .as_ref()
        .ok_or_else(|| anyhow!("Missing PTB effects for debug treasury init"))?;
    sync_dynamic_field_entries(state, effects);
    tracing::info!(
        "Router: debug treasury init effects created={} mutated={} object_changes={}",
        effects.created.len(),
        effects.mutated.len(),
        effects.object_changes.len()
    );

    // init_for_router returns TreasuryCap<DEBUG_TOKEN>; sandbox currently does not
    // always surface it in object_changes, so recover from command return bytes.
    let treasury_from_return = effects
        .return_values
        .first()
        .and_then(|values| values.first())
        .cloned();

    let treasury_id = if let Some(cap_bytes) = treasury_from_return {
        if cap_bytes.len() < AccountAddress::LENGTH {
            return Err(anyhow!(
                "debug treasury init returned short TreasuryCap bytes: {}",
                cap_bytes.len()
            ));
        }
        let mut id_bytes = [0u8; AccountAddress::LENGTH];
        id_bytes.copy_from_slice(&cap_bytes[..AccountAddress::LENGTH]);
        let treasury_id = AccountAddress::new(id_bytes);
        if state.env.get_object(&treasury_id).is_none() {
            state.env.load_object_from_data(
                &treasury_id.to_hex_literal(),
                cap_bytes,
                Some(DEBUG_TREASURY_TYPE),
                false,
                false,
                1,
            )?;
        }
        treasury_id
    } else {
        find_created_object_id_by_type(effects, DEBUG_TREASURY_TYPE)
            .or_else(|| {
                state
                    .env
                    .list_objects()
                    .into_iter()
                    .find(|obj| obj.type_tag == treasury_tag)
                    .map(|obj| obj.id)
            })
            .ok_or_else(|| {
                let matching: Vec<String> = state
                    .env
                    .list_objects()
                    .into_iter()
                    .filter(|obj| obj.type_tag.to_string().contains("::debug_token::"))
                    .map(|obj| format!("{}:{}", obj.id, obj.type_tag))
                    .collect();
                anyhow!(
                    "Could not locate debug treasury cap object after init_for_router (debug objects in env: [{}])",
                    matching.join(", ")
                )
            })?
    };

    state.debug_treasury_id = Some(treasury_id);
    tracing::info!(
        "Router: debug treasury ready in VM at {}",
        treasury_id.to_hex_literal()
    );
    Ok(treasury_id)
}

fn debug_treasury_shared_input(state: &RouterEnvState, treasury_id: AccountAddress) -> Result<ObjectInput> {
    let treasury_obj = state
        .env
        .get_object(&treasury_id)
        .ok_or_else(|| anyhow!("Debug treasury cap object missing in env: {}", treasury_id))?;

    Ok(ObjectInput::Owned {
        id: treasury_id,
        bytes: treasury_obj.bcs_bytes.clone(),
        type_tag: Some(TypeTag::from_str(DEBUG_TREASURY_TYPE)?),
        version: Some(treasury_obj.version),
    })
}

fn mint_debug_reserve_coin(state: &mut RouterEnvState, amount: u64) -> Result<AccountAddress> {
    let treasury_id = ensure_debug_treasury(state)?;
    let sui_framework_addr = AccountAddress::from_hex_literal(SUI_FRAMEWORK_PACKAGE)?;

    let inputs = vec![
        InputValue::Object(debug_treasury_shared_input(state, treasury_id)?),
        InputValue::Pure(bcs::to_bytes(&amount)?),
    ];
    let commands = vec![
        Command::MoveCall {
            package: sui_framework_addr,
            module: Identifier::new("coin")?,
            function: Identifier::new("mint")?,
            type_args: vec![TypeTag::from_str(DEBUG_TYPE)?],
            args: vec![Argument::Input(0), Argument::Input(1)],
        },
        Command::MoveCall {
            package: sui_framework_addr,
            module: Identifier::new("coin")?,
            function: Identifier::new("value")?,
            type_args: vec![TypeTag::from_str(DEBUG_TYPE)?],
            args: vec![Argument::Result(0)],
        },
    ];

    let result = state.env.execute_ptb(inputs, commands);
    if !result.success {
        return Err(anyhow!(
            "debug reserve mint failed: {}",
            result
                .raw_error
                .unwrap_or_else(|| "Unknown error".to_string())
        ));
    }
    let effects = result
        .effects
        .as_ref()
        .ok_or_else(|| anyhow!("Missing PTB effects for debug reserve mint"))?;
    sync_dynamic_field_entries(state, effects);

    let minted = parse_u64_command_return(effects, 1, 0, "debug_minted_amount")?;
    if minted != amount {
        return Err(anyhow!(
            "debug reserve mint mismatch: requested {}, minted {}",
            amount,
            minted
        ));
    }

    let debug_coin_type = coin_object_type(DEBUG_TYPE);
    let reserve_id = find_created_object_id_by_type(effects, &debug_coin_type)
        .ok_or_else(|| anyhow!("Could not locate created DEBUG coin from mint PTB effects"))?;

    if state.env.get_object(&reserve_id).is_none() {
        if let Some(bytes) = effects.created_object_bytes.get(&reserve_id) {
            state.env.load_object_from_data(
                &reserve_id.to_hex_literal(),
                bytes.clone(),
                Some(&debug_coin_type),
                false,
                false,
                1,
            )?;
        }
    }

    state
        .coin_reserve_cache
        .insert(DEBUG_TYPE.to_string(), reserve_id);
    tracing::info!(
        "Router: DEBUG reserve minted in VM at {} (amount={})",
        reserve_id.to_hex_literal(),
        amount
    );
    Ok(reserve_id)
}

fn reserve_coin_input(state: &mut RouterEnvState, coin_type: &str) -> Result<ObjectInput> {
    let reserve_id = if let Some(existing) = state.coin_reserve_cache.get(coin_type) {
        *existing
    } else if coin_type == DEBUG_TYPE {
        mint_debug_reserve_coin(state, RESERVE_COIN_SEED_AMOUNT)?
    } else {
        return Err(anyhow!(
            "VM reserve coin missing for {}. Expected checkpoint-backed reserve bootstrap during setup.",
            coin_type
        ));
    };

    let reserve_obj = state
        .env
        .get_object(&reserve_id)
        .ok_or_else(|| anyhow!("VM reserve coin missing in env: {}", reserve_id))?;

    Ok(ObjectInput::Owned {
        id: reserve_id,
        bytes: reserve_obj.bcs_bytes.clone(),
        type_tag: Some(reserve_obj.type_tag.clone()),
        version: Some(reserve_obj.version),
    })
}

fn collect_swap_events(effects: &sui_sandbox_core::ptb::TransactionEffects) -> Vec<SwapEvent> {
    effects
        .events
        .iter()
        .map(|event| SwapEvent {
            event_type: event.type_tag.clone(),
            data_hex: hex::encode(&event.data),
        })
        .collect()
}

fn read_uleb128(cursor: &mut std::io::Cursor<&[u8]>) -> Result<u64> {
    let mut value = 0u64;
    let mut shift = 0u32;

    loop {
        let mut byte = [0u8; 1];
        cursor
            .read_exact(&mut byte)
            .map_err(|e| anyhow!("Failed reading ULEB128: {}", e))?;
        let b = byte[0];
        value |= ((b & 0x7f) as u64) << shift;

        if (b & 0x80) == 0 {
            break;
        }

        shift += 7;
        if shift >= 64 {
            return Err(anyhow!("ULEB128 value too large"));
        }
    }

    Ok(value)
}

fn read_u64_le(cursor: &mut std::io::Cursor<&[u8]>, field: &str) -> Result<u64> {
    let mut bytes = [0u8; 8];
    cursor
        .read_exact(&mut bytes)
        .map_err(|e| anyhow!("Failed reading {}: {}", field, e))?;
    Ok(u64::from_le_bytes(bytes))
}

fn read_u128_le(cursor: &mut std::io::Cursor<&[u8]>, field: &str) -> Result<u128> {
    let mut bytes = [0u8; 16];
    cursor
        .read_exact(&mut bytes)
        .map_err(|e| anyhow!("Failed reading {}: {}", field, e))?;
    Ok(u128::from_le_bytes(bytes))
}

#[derive(Debug, Clone)]
struct OrderPageSummary {
    order_count: usize,
    has_next_page: bool,
    first_order_id: Option<u128>,
    first_price: Option<u64>,
    first_quantity: Option<u64>,
    first_filled_quantity: Option<u64>,
    first_status: Option<u8>,
}

fn parse_order_page_summary(bytes: &[u8]) -> Result<OrderPageSummary> {
    let mut cursor = std::io::Cursor::new(bytes);
    let order_count = read_uleb128(&mut cursor)? as usize;

    let mut first_order_id = None;
    let mut first_price = None;
    let mut first_quantity = None;
    let mut first_filled_quantity = None;
    let mut first_status = None;

    for idx in 0..order_count {
        // balance_manager_id
        let mut skip_32 = [0u8; 32];
        cursor
            .read_exact(&mut skip_32)
            .map_err(|e| anyhow!("Failed reading order[{}].balance_manager_id: {}", idx, e))?;

        let order_id = read_u128_le(&mut cursor, "order_id")?;
        let _client_order_id = read_u64_le(&mut cursor, "client_order_id")?;
        let quantity = read_u64_le(&mut cursor, "quantity")?;
        let filled_quantity = read_u64_le(&mut cursor, "filled_quantity")?;

        // fee_is_deep + order_deep_price.asset_is_base
        let mut skip_2 = [0u8; 2];
        cursor
            .read_exact(&mut skip_2)
            .map_err(|e| anyhow!("Failed reading order[{}] flags: {}", idx, e))?;

        // order_deep_price.deep_per_asset
        let price = read_u64_le(&mut cursor, "order_deep_price.deep_per_asset")?;
        let _epoch = read_u64_le(&mut cursor, "epoch")?;

        let mut status = [0u8; 1];
        cursor
            .read_exact(&mut status)
            .map_err(|e| anyhow!("Failed reading order[{}].status: {}", idx, e))?;
        let _expire = read_u64_le(&mut cursor, "expire_timestamp")?;

        if idx == 0 {
            first_order_id = Some(order_id);
            first_price = Some(price);
            first_quantity = Some(quantity);
            first_filled_quantity = Some(filled_quantity);
            first_status = Some(status[0]);
        }
    }

    let mut has_next = [0u8; 1];
    cursor
        .read_exact(&mut has_next)
        .map_err(|e| anyhow!("Failed reading has_next_page: {}", e))?;

    Ok(OrderPageSummary {
        order_count,
        has_next_page: has_next[0] != 0,
        first_order_id,
        first_price,
        first_quantity,
        first_filled_quantity,
        first_status,
    })
}

fn fetch_debug_iter_orders_summary(
    state: &mut RouterEnvState,
    bids: bool,
    limit: u64,
) -> Result<OrderPageSummary> {
    let deepbook_addr = AccountAddress::from_hex_literal(DEEPBOOK_PACKAGE)?;
    let debug_tag = TypeTag::from_str(DEBUG_TYPE)?;
    let usdc_tag = TypeTag::from_str(USDC_TYPE)?;

    let inputs = vec![
        InputValue::Object(pool_shared_input(state, PoolId::DebugUsdc, false)?),
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
        type_args: vec![debug_tag, usdc_tag],
        args: vec![
            Argument::Input(0),
            Argument::Input(1),
            Argument::Input(2),
            Argument::Input(3),
            Argument::Input(4),
            Argument::Input(5),
        ],
    }];

    let result = state.env.execute_ptb(inputs, commands);
    if !result.success {
        return Err(anyhow!(
            "debug iter_orders({}) failed: {}",
            if bids { "bids" } else { "asks" },
            result
                .raw_error
                .unwrap_or_else(|| "Unknown error".to_string())
        ));
    }

    let return_bytes = result
        .effects
        .as_ref()
        .and_then(|effects| effects.return_values.first())
        .and_then(|cmd_returns| cmd_returns.first().cloned())
        .ok_or_else(|| anyhow!("No return values from debug iter_orders"))?;

    parse_order_page_summary(&return_bytes)
}

fn log_debug_pool_snapshot(state: &mut RouterEnvState, context: &str) -> Result<()> {
    let deepbook_addr = AccountAddress::from_hex_literal(DEEPBOOK_PACKAGE)?;
    let debug_tag = TypeTag::from_str(DEBUG_TYPE)?;
    let usdc_tag = TypeTag::from_str(USDC_TYPE)?;
    let ticks: u64 = 5;

    let inputs = vec![
        InputValue::Object(pool_shared_input(state, PoolId::DebugUsdc, false)?),
        InputValue::Pure(bcs::to_bytes(&ticks)?),
        InputValue::Object(state.next_clock_input()?),
    ];

    let commands = vec![
        Command::MoveCall {
            package: deepbook_addr,
            module: Identifier::new("pool")?,
            function: Identifier::new("pool_book_params")?,
            type_args: vec![debug_tag.clone(), usdc_tag.clone()],
            args: vec![Argument::Input(0)],
        },
        Command::MoveCall {
            package: deepbook_addr,
            module: Identifier::new("pool")?,
            function: Identifier::new("whitelisted")?,
            type_args: vec![debug_tag.clone(), usdc_tag.clone()],
            args: vec![Argument::Input(0)],
        },
        Command::MoveCall {
            package: deepbook_addr,
            module: Identifier::new("pool")?,
            function: Identifier::new("registered_pool")?,
            type_args: vec![debug_tag.clone(), usdc_tag.clone()],
            args: vec![Argument::Input(0)],
        },
        Command::MoveCall {
            package: deepbook_addr,
            module: Identifier::new("pool")?,
            function: Identifier::new("vault_balances")?,
            type_args: vec![debug_tag.clone(), usdc_tag.clone()],
            args: vec![Argument::Input(0)],
        },
        Command::MoveCall {
            package: deepbook_addr,
            module: Identifier::new("pool")?,
            function: Identifier::new("get_level2_ticks_from_mid")?,
            type_args: vec![debug_tag, usdc_tag],
            args: vec![Argument::Input(0), Argument::Input(1), Argument::Input(2)],
        },
    ];

    let result = state.env.execute_ptb(inputs, commands);
    if !result.success {
        return Err(anyhow!(
            "debug snapshot PTB failed ({}): {}",
            context,
            result
                .raw_error
                .unwrap_or_else(|| "Unknown error".to_string())
        ));
    }

    let effects = result
        .effects
        .as_ref()
        .ok_or_else(|| anyhow!("Missing PTB effects for debug snapshot ({})", context))?;
    sync_dynamic_field_entries(state, effects);

    let tick_size = parse_u64_command_return(effects, 0, 0, "tick_size")?;
    let lot_size = parse_u64_command_return(effects, 0, 1, "lot_size")?;
    let min_size = parse_u64_command_return(effects, 0, 2, "min_size")?;
    let whitelisted = parse_bool_command_return(effects, 1, 0, "whitelisted")?;
    let registered_pool = parse_bool_command_return(effects, 2, 0, "registered_pool")?;
    let vault_base = parse_u64_command_return(effects, 3, 0, "vault_base")?;
    let vault_quote = parse_u64_command_return(effects, 3, 1, "vault_quote")?;
    let vault_deep = parse_u64_command_return(effects, 3, 2, "vault_deep")?;

    let bid_prices = parse_vec_u64_command_return(effects, 4, 0, "bid_prices")?;
    let bid_quantities = parse_vec_u64_command_return(effects, 4, 1, "bid_quantities")?;
    let ask_prices = parse_vec_u64_command_return(effects, 4, 2, "ask_prices")?;
    let ask_quantities = parse_vec_u64_command_return(effects, 4, 3, "ask_quantities")?;
    let iter_bids = fetch_debug_iter_orders_summary(state, true, 10)?;
    let iter_asks = fetch_debug_iter_orders_summary(state, false, 10)?;

    tracing::info!(
        "Router: debug snapshot [{}] whitelisted={}, registered_pool={}, tick_size={}, lot_size={}, min_size={}, vault(base={}, quote={}, deep={}), l2_bid_levels={}, l2_ask_levels={}, l2_best_bid={:?}/{:?}, l2_best_ask={:?}/{:?}, iter_bid_count={}, iter_ask_count={}, iter_first_bid={:?}/{:?}/{:?}/{:?}/{:?}, iter_first_ask={:?}/{:?}/{:?}/{:?}/{:?}, iter_has_next_bid={}, iter_has_next_ask={}",
        context,
        whitelisted,
        registered_pool,
        tick_size,
        lot_size,
        min_size,
        vault_base,
        vault_quote,
        vault_deep,
        bid_prices.len(),
        ask_prices.len(),
        bid_prices.first(),
        bid_quantities.first(),
        ask_prices.first(),
        ask_quantities.first(),
        iter_bids.order_count,
        iter_asks.order_count,
        iter_bids.first_order_id,
        iter_bids.first_price,
        iter_bids.first_quantity,
        iter_bids.first_filled_quantity,
        iter_bids.first_status,
        iter_asks.first_order_id,
        iter_asks.first_price,
        iter_asks.first_quantity,
        iter_asks.first_filled_quantity,
        iter_asks.first_status,
        iter_bids.has_next_page,
        iter_asks.has_next_page
    );

    Ok(())
}

fn execute_single_hop_quote(
    state: &mut RouterEnvState,
    pool_id: PoolId,
    input_amount: u64,
    is_sell_base: bool,
) -> Result<SingleHopQuote> {
    let (base_type, quote_type) = pool_types(pool_id);
    let base_tag = TypeTag::from_str(base_type)?;
    let quote_tag = TypeTag::from_str(quote_type)?;
    let deepbook_addr = AccountAddress::from_hex_literal(DEEPBOOK_PACKAGE)?;
    let function_name = if is_sell_base {
        "get_quote_quantity_out"
    } else {
        "get_base_quantity_out"
    };

    let inputs = vec![
        InputValue::Object(pool_shared_input(state, pool_id, false)?),
        InputValue::Pure(bcs::to_bytes(&input_amount)?),
        InputValue::Object(state.next_clock_input()?),
    ];

    let commands = vec![Command::MoveCall {
        package: deepbook_addr,
        module: Identifier::new("pool")?,
        function: Identifier::new(function_name)?,
        type_args: vec![base_tag, quote_tag],
        args: vec![Argument::Input(0), Argument::Input(1), Argument::Input(2)],
    }];

    let result = state.env.execute_ptb(inputs, commands);

    if !result.success {
        return Err(anyhow!(
            "single-hop quote via pool::{} failed for {}: {}",
            function_name,
            pool_id.display_name(),
            result
                .raw_error
                .unwrap_or_else(|| "Unknown error".to_string())
        ));
    }

    let return_values = result
        .effects
        .as_ref()
        .and_then(|effects| effects.return_values.first())
        .ok_or_else(|| anyhow!("No return values from pool::{}", function_name))?;

    let rv0 = parse_u64_return(return_values, 0, "rv0")?;
    let rv1 = parse_u64_return(return_values, 1, "rv1")?;
    let rv2 = parse_u64_return(return_values, 2, "rv2")?;
    if pool_id == PoolId::DebugUsdc {
        tracing::info!(
            "Router: debug quote {} returns rv0={}, rv1={}, rv2={}, input={}",
            function_name,
            rv0,
            rv1,
            rv2,
            input_amount
        );
    }

    let output_amount = if is_sell_base {
        // get_quote_quantity_out returns (base_left, quote_out, deep_fee)
        rv1
    } else {
        // get_base_quantity_out returns (base_out, quote_left, deep_fee)
        rv0
    };
    if pool_id == PoolId::DebugUsdc && output_amount == 0 {
        if let Err(e) = log_debug_pool_snapshot(state, "quote-zero-output") {
            tracing::warn!("Router: debug snapshot failed after zero quote output: {}", e);
        }
    }

    Ok(SingleHopQuote { output_amount })
}

fn log_debug_order_lookup(state: &mut RouterEnvState, context: &str, order_id: u128) -> Result<()> {
    let deepbook_addr = AccountAddress::from_hex_literal(DEEPBOOK_PACKAGE)?;
    let debug_tag = TypeTag::from_str(DEBUG_TYPE)?;
    let usdc_tag = TypeTag::from_str(USDC_TYPE)?;

    let inputs = vec![
        InputValue::Object(pool_shared_input(state, PoolId::DebugUsdc, false)?),
        InputValue::Pure(bcs::to_bytes(&order_id)?),
    ];
    let commands = vec![
        Command::MoveCall {
            package: deepbook_addr,
            module: Identifier::new("pool")?,
            function: Identifier::new("get_order")?,
            type_args: vec![debug_tag, usdc_tag],
            args: vec![Argument::Input(0), Argument::Input(1)],
        },
        Command::MoveCall {
            package: deepbook_addr,
            module: Identifier::new("order")?,
            function: Identifier::new("price")?,
            type_args: vec![],
            args: vec![Argument::NestedResult(0, 0)],
        },
        Command::MoveCall {
            package: deepbook_addr,
            module: Identifier::new("order")?,
            function: Identifier::new("quantity")?,
            type_args: vec![],
            args: vec![Argument::NestedResult(0, 0)],
        },
        Command::MoveCall {
            package: deepbook_addr,
            module: Identifier::new("order")?,
            function: Identifier::new("filled_quantity")?,
            type_args: vec![],
            args: vec![Argument::NestedResult(0, 0)],
        },
        Command::MoveCall {
            package: deepbook_addr,
            module: Identifier::new("order")?,
            function: Identifier::new("status")?,
            type_args: vec![],
            args: vec![Argument::NestedResult(0, 0)],
        },
        Command::MoveCall {
            package: deepbook_addr,
            module: Identifier::new("order")?,
            function: Identifier::new("expire_timestamp")?,
            type_args: vec![],
            args: vec![Argument::NestedResult(0, 0)],
        },
    ];

    let result = state.env.execute_ptb(inputs, commands);
    if !result.success {
        if let Some(ctx) = result.error_context.as_ref() {
            tracing::warn!("Router: debug get_order error_context [{}]: {:?}", context, ctx);
        }
        if let Some(snapshot) = result.state_at_failure.as_ref() {
            tracing::warn!(
                "Router: debug get_order state_at_failure [{}]: dynamic_fields_accessed={:?}",
                context,
                snapshot.dynamic_fields_accessed
            );
        }
        return Err(anyhow!(
            "debug get_order lookup failed [{}] for order_id {}: {}",
            context,
            order_id,
            result
                .raw_error
                .unwrap_or_else(|| "Unknown error".to_string())
        ));
    }

    let effects = result
        .effects
        .as_ref()
        .ok_or_else(|| anyhow!("Missing PTB effects for debug get_order lookup"))?;
    let price = parse_u64_command_return(effects, 1, 0, "order.price")?;
    let quantity = parse_u64_command_return(effects, 2, 0, "order.quantity")?;
    let filled_quantity = parse_u64_command_return(effects, 3, 0, "order.filled_quantity")?;
    let status = parse_u8_command_return(effects, 4, 0, "order.status")?;
    let expire_timestamp = parse_u64_command_return(effects, 5, 0, "order.expire_timestamp")?;
    tracing::info!(
        "Router: debug get_order [{}] order_id={} price={} qty={} filled={} status={} expire={}",
        context,
        order_id,
        price,
        quantity,
        filled_quantity,
        status,
        expire_timestamp
    );

    Ok(())
}

/// Create a synthetic Clock object at address 0x6
fn create_clock_object(env: &mut SimulationEnvironment, timestamp_ms: u64) -> Result<()> {
    // Clock struct in BCS: UID (32 bytes) + timestamp_ms (u64)
    // UID is the object ID padded to 32 bytes
    let clock_addr = AccountAddress::from_hex_literal(CLOCK_OBJECT_ID)?;
    let mut bcs_bytes = Vec::new();
    bcs_bytes.extend_from_slice(clock_addr.as_ref()); // UID = 32 bytes
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
    let router_dir = resolve_router_contract_dir()?;

    tracing::info!("Router: compiling router contract...");

    // Compile against mainnet dependency addresses so router bytecode links to
    // the same DeepBook package loaded into the simulation environment.
    // Fall back to default build for older CLI/environment setups.
    let mainnet_build = run_sui_move_build(
        &router_dir,
        &["move", "build", "--environment", "mainnet", "--force"],
    );
    if let Err(mainnet_err) = mainnet_build {
        tracing::warn!(
            "Router: `sui move build --environment mainnet` failed, trying default build:\n{}",
            mainnet_err
        );
        run_sui_move_build(&router_dir, &["move", "build", "--force"]).map_err(|fallback_err| {
            anyhow!(
                "Router compile failed for both mainnet and default builds.\nMainnet build error:\n{}\nFallback build error:\n{}",
                mainnet_err,
                fallback_err
            )
        })?;
    }
    tracing::info!("Router: contract compiled successfully");

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
    tracing::info!(
        "Router: deployed router contract at {}",
        ROUTER_PACKAGE_ADDR
    );

    Ok(())
}

fn resolve_router_contract_dir() -> Result<PathBuf> {
    // Primary resolution based on crate location (works regardless of process cwd).
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let rooted_router_dir = manifest_dir.join("../contracts/router");
    if rooted_router_dir.exists() {
        return Ok(rooted_router_dir);
    }

    // Backwards-compatible fallbacks for ad-hoc runs.
    let cwd_router_dir = Path::new("./contracts/router");
    if cwd_router_dir.exists() {
        return Ok(cwd_router_dir.to_path_buf());
    }

    let parent_router_dir = Path::new("../contracts/router");
    if parent_router_dir.exists() {
        return Ok(parent_router_dir.to_path_buf());
    }

    Err(anyhow!(
        "Router contract directory not found. Checked: {}, ./contracts/router, ../contracts/router",
        rooted_router_dir.display()
    ))
}

fn run_sui_move_build(router_dir: &Path, args: &[&str]) -> Result<()> {
    let output = std::process::Command::new("sui")
        .args(args)
        .current_dir(router_dir)
        .output()
        .map_err(|e| anyhow!("Failed to run `sui {}`: {}", args.join(" "), e))?;

    if output.status.success() {
        return Ok(());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(anyhow!(
        "`sui {}` failed (status: {}).\nstdout:\n{}\nstderr:\n{}",
        args.join(" "),
        output
            .status
            .code()
            .map(|c| c.to_string())
            .unwrap_or_else(|| "signal".to_string()),
        stdout,
        stderr
    ))
}

fn run_router_health_check(state: &mut RouterEnvState) -> Result<()> {
    // Prefer SUI -> WAL path, then SUI -> DEEP, then WAL -> DEEP.
    let candidates = [
        (PoolId::SuiUsdc, PoolId::WalUsdc),
        (PoolId::SuiUsdc, PoolId::DeepUsdc),
        (PoolId::WalUsdc, PoolId::DeepUsdc),
    ];
    // DeepBook can abort on dust-sized quote amounts. Probe with practical sizes.
    let probe_amounts = [5_000_000_000_u64, 1_000_000_000, 500_000_000, 100_000_000];
    let mut last_err: Option<anyhow::Error> = None;

    for (from_pool, to_pool) in candidates {
        if !state.pool_cache.contains_key(&from_pool) || !state.pool_cache.contains_key(&to_pool) {
            continue;
        }

        for amount in probe_amounts {
            match execute_two_hop_quote(state, from_pool, to_pool, amount) {
                Ok(_) => {
                    tracing::info!(
                        "Router: health check passed via quote_two_hop ({} -> {}, probe={})",
                        from_pool.display_name(),
                        to_pool.display_name(),
                        amount
                    );
                    return Ok(());
                }
                Err(e) => {
                    last_err = Some(anyhow!(
                        "Router health check failed for {} -> {} (probe={}): {}",
                        from_pool.display_name(),
                        to_pool.display_name(),
                        amount,
                        e
                    ));
                }
            }
        }
    }

    if let Some(err) = last_err {
        return Err(err);
    }

    Err(anyhow!(
        "Router health check could not run: at least two pool states are required"
    ))
}

fn now_unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn run_startup_self_check(state: &mut RouterEnvState) -> Result<RouterStartupCheckReport> {
    let mut errors = Vec::new();

    if !state.router_deployed {
        errors.push("Router package deployment flag is false".to_string());
    }

    let mut shared_objects = Vec::new();
    for (name, object_id) in [
        ("Sui Coin Registry", COIN_REGISTRY_OBJECT_ID),
        ("DeepBook Registry", DEEPBOOK_REGISTRY_ID),
        ("Clock", CLOCK_OBJECT_ID),
    ] {
        let addr = AccountAddress::from_hex_literal(object_id)?;
        let obj = state.env.get_object(&addr);
        let present = obj.is_some();
        let is_shared = obj.map(|o| o.is_shared).unwrap_or(false);
        let version = obj.map(|o| o.version);

        if !present {
            errors.push(format!(
                "Missing required shared object in VM: {} ({})",
                name, object_id
            ));
        } else if !is_shared {
            errors.push(format!(
                "Required object is not shared in VM: {} ({})",
                name, object_id
            ));
        }

        shared_objects.push(RouterSharedObjectCheck {
            name: name.to_string(),
            object_id: object_id.to_string(),
            present,
            is_shared,
            version,
        });
    }

    let mut reserve_coins = Vec::new();
    for coin_type in [SUI_TYPE, USDC_TYPE, WAL_TYPE, DEEP_TYPE] {
        let reserve_id = state.coin_reserve_cache.get(coin_type).copied();
        let reserve_obj = reserve_id.and_then(|id| state.env.get_object(&id));
        let present = reserve_obj.is_some();
        let version = reserve_obj.map(|obj| obj.version);
        let value = reserve_obj.and_then(|obj| parse_coin_value_from_bcs(&obj.bcs_bytes));

        if reserve_id.is_none() {
            errors.push(format!(
                "Reserve bootstrap missing entry for coin type {}",
                coin_type
            ));
        } else if !present {
            errors.push(format!(
                "Reserve bootstrap object missing in VM for coin type {}",
                coin_type
            ));
        } else if value.unwrap_or(0) == 0 {
            errors.push(format!(
                "Reserve coin value is zero for coin type {}",
                coin_type
            ));
        }

        reserve_coins.push(RouterReserveCoinCheck {
            coin_type: coin_type.to_string(),
            object_id: reserve_id.map(|id| id.to_hex_literal()),
            present,
            version,
            value,
        });
    }

    let router_health_check_passed = match run_router_health_check(state) {
        Ok(()) => true,
        Err(e) => {
            errors.push(format!("Router health check failed: {}", e));
            false
        }
    };

    let report = RouterStartupCheckReport {
        ok: errors.is_empty() && state.router_deployed && router_health_check_passed,
        checked_at_unix_ms: now_unix_ms(),
        router_package_deployed: state.router_deployed,
        router_health_check_passed,
        shared_objects,
        reserve_coins,
        errors,
    };

    if report.ok {
        tracing::info!("Router startup self-check passed");
        return Ok(report);
    }

    Err(anyhow!(
        "Router startup self-check failed: {}",
        report.errors.join(" | ")
    ))
}

fn ensure_debug_pool(state: &mut RouterEnvState) -> Result<DebugPoolInfo> {
    if let Some(existing) = state.debug_pool_info.clone() {
        return Ok(existing);
    }

    let config = state.debug_pool_config.clone();
    ensure_debug_pool_with_config(state, config)
}

fn ensure_debug_pool_with_config(
    state: &mut RouterEnvState,
    mut config: DebugPoolCreateConfig,
) -> Result<DebugPoolInfo> {
    config.token_symbol = config.token_symbol.trim().to_uppercase();
    config.token_name = config.token_name.trim().to_string();
    config.token_description = config.token_description.trim().to_string();
    config.token_icon_url = config.token_icon_url.trim().to_string();
    config.token_decimals = 9;

    if config.token_symbol.is_empty() {
        return Err(anyhow!("token_symbol is required"));
    }
    if config.token_symbol.len() > 12 {
        return Err(anyhow!("token_symbol must be <= 12 chars"));
    }
    if config.token_name.is_empty() {
        config.token_name = config.token_symbol.clone();
    }
    if config.token_name.len() > 64 {
        return Err(anyhow!("token_name must be <= 64 chars"));
    }
    if config.token_description.len() > 256 {
        return Err(anyhow!("token_description must be <= 256 chars"));
    }
    if let Some(existing) = state.debug_pool_info.clone() {
        if existing.config != config {
            return Err(anyhow!(
                "debug pool already exists with token_symbol={} and different config; restart backend to apply new debug pool config",
                existing.token_symbol
            ));
        }
        return Ok(existing);
    }

    state.debug_pool_config = config.clone();

    if let Some(existing) = state.pool_cache.get(&PoolId::DebugUsdc) {
        let info = DebugPoolInfo {
            pool_object_id: existing.pool_addr.to_hex_literal(),
            token_symbol: config.token_symbol.clone(),
            token_type: DEBUG_TYPE.to_string(),
            config,
        };
        state.debug_pool_info = Some(info.clone());
        return Ok(info);
    }

    tracing::info!(
        "Router: creating debug pool {}/USDC in local VM...",
        config.token_symbol
    );
    create_debug_pool(state, &config)?;
    seed_debug_pool_orderbook(state, &config)?;

    let entry = state
        .pool_cache
        .get(&PoolId::DebugUsdc)
        .ok_or_else(|| anyhow!("Debug pool missing from router cache after creation"))?;
    tracing::info!(
        "Router: debug pool ready at {} (type {})",
        entry.pool_addr,
        DEBUG_TYPE
    );

    let info = DebugPoolInfo {
        pool_object_id: entry.pool_addr.to_hex_literal(),
        token_symbol: config.token_symbol.clone(),
        token_type: DEBUG_TYPE.to_string(),
        config,
    };
    state.debug_pool_info = Some(info.clone());
    Ok(info)
}

fn create_debug_pool(state: &mut RouterEnvState, config: &DebugPoolCreateConfig) -> Result<()> {
    let deepbook_addr = AccountAddress::from_hex_literal(DEEPBOOK_PACKAGE)?;
    let debug_tag = TypeTag::from_str(DEBUG_TYPE)?;
    let usdc_tag = TypeTag::from_str(USDC_TYPE)?;
    let pool_type = build_pool_type_tag(DEBUG_TYPE, USDC_TYPE)?;
    let existing_pool_ids: HashSet<AccountAddress> = state
        .env
        .list_objects()
        .into_iter()
        .filter(|obj| obj.type_tag == pool_type)
        .map(|obj| obj.id)
        .collect();
    ensure_debug_admin_cap(state)?;

    let inputs = vec![
        // Input 0: DeepBook Registry (shared mutable)
        InputValue::Object(registry_shared_input(state, true)?),
        // Input 1: tick_size
        InputValue::Pure(bcs::to_bytes(&config.tick_size)?),
        // Input 2: lot_size
        InputValue::Pure(bcs::to_bytes(&config.lot_size)?),
        // Input 3: min_size
        InputValue::Pure(bcs::to_bytes(&config.min_size)?),
        // Input 4: whitelisted_pool
        InputValue::Pure(bcs::to_bytes(&config.whitelisted_pool)?),
        // Input 5: stable_pool
        InputValue::Pure(bcs::to_bytes(&false)?),
        // Input 6: admin cap
        InputValue::Object(admin_cap_input(state)?),
    ];

    let commands = vec![Command::MoveCall {
        package: deepbook_addr,
        module: Identifier::new("pool")?,
        function: Identifier::new("create_pool_admin")?,
        type_args: vec![debug_tag, usdc_tag],
        args: vec![
            Argument::Input(0),
            Argument::Input(1),
            Argument::Input(2),
            Argument::Input(3),
            Argument::Input(4),
            Argument::Input(5),
            Argument::Input(6),
        ],
    }];

    let result = state.env.execute_ptb(inputs, commands);
    if !result.success {
        return Err(anyhow!(
            "debug pool creation failed: {}",
            result
                .raw_error
                .unwrap_or_else(|| "Unknown error".to_string())
        ));
    }
    let effects = result
        .effects
        .as_ref()
        .ok_or_else(|| anyhow!("Missing effects from debug pool creation"))?;
    sync_dynamic_field_entries(state, effects);
    {
        tracing::info!(
            "Router: debug pool create effects -> created={}, dynamic_fields={}",
            effects.created.len(),
            effects.dynamic_field_entries.len()
        );
        for created_id in &effects.created {
            if let Some(obj) = state.env.get_object(created_id) {
                tracing::info!(
                    "Router: created object {} type {} (shared={})",
                    created_id,
                    obj.type_tag,
                    obj.is_shared
                );
            } else {
                tracing::warn!(
                    "Router: created object {} not present in env after PTB",
                    created_id
                );
            }
        }
    }

    let pool_addr = effects
        .return_values
        .first()
        .and_then(|values| values.first())
        .and_then(|bytes| {
            bcs::from_bytes::<AccountAddress>(bytes).ok().or_else(|| {
                if bytes.len() >= AccountAddress::LENGTH {
                    let mut raw = [0u8; AccountAddress::LENGTH];
                    raw.copy_from_slice(&bytes[..AccountAddress::LENGTH]);
                    Some(AccountAddress::new(raw))
                } else {
                    None
                }
            })
        })
        .ok_or_else(|| anyhow!("Failed to decode debug pool id from PTB return values"))?;

    if let Some(wrapper_bytes) = effects.created_object_bytes.get(&pool_addr) {
        if state.env.get_object(&pool_addr).is_some() {
            state.env.set_object_bytes(pool_addr, wrapper_bytes.clone()).map_err(|e| {
                anyhow!(
                    "failed updating created DBG/USDC pool wrapper {} bytes: {}",
                    pool_addr,
                    e
                )
            })?;
        } else {
            state.env.load_object_from_data(
                &pool_addr.to_hex_literal(),
                wrapper_bytes.clone(),
                Some(&format!(
                    "{}::pool::Pool<{},{}>",
                    DEEPBOOK_PACKAGE, DEBUG_TYPE, USDC_TYPE
                )),
                true,
                false,
                0,
            )?;
            tracing::info!(
                "Router: loaded DBG/USDC pool wrapper {} directly from create effects",
                pool_addr
            );
        }
    }

    // Some sandbox versions fail to materialize the shared pool wrapper object even
    // when the create PTB succeeds. Recover by synthesizing the wrapper from the
    // returned pool ID and the created PoolInner dynamic-field parent.
    if state.env.get_object(&pool_addr).is_none() {
        let pool_inner_parent = effects.dynamic_field_entries.iter().find_map(
            |((parent_id, _child_id), (type_tag, _bytes))| {
                let tag = type_tag.to_string();
                if tag.contains("::pool::PoolInner<")
                    && tag.contains(DEBUG_TYPE)
                    && tag.contains(USDC_TYPE)
                {
                    Some(*parent_id)
                } else {
                    None
                }
            },
        );

        if let Some(inner_parent) = pool_inner_parent {
            let mut wrapper_bytes = Vec::with_capacity(AccountAddress::LENGTH * 2 + 8);
            // Pool.id: UID
            wrapper_bytes.extend_from_slice(pool_addr.as_ref());
            // Pool.inner.id: UID
            wrapper_bytes.extend_from_slice(inner_parent.as_ref());
            // Pool.inner.version
            wrapper_bytes.extend_from_slice(&1_u64.to_le_bytes());

            state.env.load_object_from_data(
                &pool_addr.to_hex_literal(),
                wrapper_bytes,
                Some(&format!(
                    "{}::pool::Pool<{},{}>",
                    DEEPBOOK_PACKAGE, DEBUG_TYPE, USDC_TYPE
                )),
                true,
                false,
                1,
            )?;
            tracing::info!(
                "Router: synthesized missing DBG/USDC pool wrapper at {} (inner={})",
                pool_addr,
                inner_parent
            );
        }
    }

    if state.env.get_object(&pool_addr).is_none() {
        return Err(anyhow!(
            "Could not locate DBG/USDC pool object after creation ({})",
            pool_addr
        ));
    }

    if existing_pool_ids.contains(&pool_addr) {
        tracing::info!(
            "Router: reusing existing DBG/USDC pool object {}",
            pool_addr
        );
    }

    state.pool_cache.insert(
        PoolId::DebugUsdc,
        PoolCacheEntry {
            pool_addr,
            pool_type,
        },
    );

    Ok(())
}

fn prime_debug_pool_deep_price(state: &mut RouterEnvState) -> Result<u64> {
    let deepbook_addr = AccountAddress::from_hex_literal(DEEPBOOK_PACKAGE)?;
    let debug_tag = TypeTag::from_str(DEBUG_TYPE)?;
    let usdc_tag = TypeTag::from_str(USDC_TYPE)?;
    let mut last_err: Option<anyhow::Error> = None;

    // Try multiple reference pools; different DeepBook versions may accept
    // different base assets for bootstrapping order deep price.
    for reference_pool in [PoolId::DeepUsdc, PoolId::SuiUsdc, PoolId::WalUsdc] {
        let (ref_base_type, _ref_quote_type) = pool_types(reference_pool);
        let ref_base_tag = TypeTag::from_str(ref_base_type)?;
        let mut points_added = 0usize;
        for _attempt in 0..3 {
            let add_inputs = vec![
                // Input 0: target DBG/USDC pool
                InputValue::Object(pool_shared_input(state, PoolId::DebugUsdc, true)?),
                // Input 1: reference */USDC pool
                InputValue::Object(pool_shared_input(state, reference_pool, false)?),
                // Input 2: clock
                InputValue::Object(state.next_clock_input()?),
            ];

            let add_commands = vec![Command::MoveCall {
                package: deepbook_addr,
                module: Identifier::new("pool")?,
                function: Identifier::new("add_deep_price_point")?,
                type_args: vec![
                    debug_tag.clone(),
                    usdc_tag.clone(),
                    ref_base_tag.clone(),
                    usdc_tag.clone(),
                ],
                args: vec![Argument::Input(0), Argument::Input(1), Argument::Input(2)],
            }];

            let add_result = state.env.execute_ptb(add_inputs, add_commands);
            if !add_result.success {
                let err = anyhow!(
                    "add_deep_price_point via {} failed: {}",
                    reference_pool.display_name(),
                    add_result
                        .raw_error
                        .unwrap_or_else(|| "Unknown error".to_string())
                );
                tracing::warn!("Router: {}", err);
                if points_added == 0 {
                    last_err = Some(err);
                }
                break;
            }
            if let Some(effects) = add_result.effects.as_ref() {
                sync_dynamic_field_entries(state, effects);
            }
            points_added += 1;
        }
        if points_added == 0 {
            continue;
        }

        // Read in a separate PTB so shared-object writes are definitely visible.
        let read_inputs = vec![InputValue::Object(pool_shared_input(
            state,
            PoolId::DebugUsdc,
            false,
        )?)];
        let read_commands = vec![
            // 0) Read current order deep price snapshot from debug pool.
            Command::MoveCall {
                package: deepbook_addr,
                module: Identifier::new("pool")?,
                function: Identifier::new("get_order_deep_price")?,
                type_args: vec![debug_tag.clone(), usdc_tag.clone()],
                args: vec![Argument::Input(0)],
            },
            // 1) Extract `deep_per_asset` from OrderDeepPrice.
            Command::MoveCall {
                package: deepbook_addr,
                module: Identifier::new("deep_price")?,
                function: Identifier::new("deep_per_asset")?,
                type_args: vec![],
                args: vec![Argument::NestedResult(0, 0)],
            },
        ];

        let result = state.env.execute_ptb(read_inputs, read_commands);
        if !result.success {
            let err = anyhow!(
                "debug pool deep_price bootstrap read failed after {}: {}",
                reference_pool.display_name(),
                result
                    .raw_error
                    .unwrap_or_else(|| "Unknown error".to_string())
            );
            tracing::warn!("Router: {}", err);
            last_err = Some(err);
            continue;
        }
        if let Some(read_effects) = result.effects.as_ref() {
            sync_dynamic_field_entries(state, read_effects);
        }

        let effects = result
            .effects
            .as_ref()
            .ok_or_else(|| anyhow!("Missing PTB effects for debug deep_price bootstrap"))?;
        let deep_per_asset = parse_u64_command_return(effects, 1, 0, "deep_per_asset")?;
        if deep_per_asset > 0 {
            tracing::info!(
                "Router: deep_price bootstrap succeeded via {} (points={}, deep_per_asset={})",
                reference_pool.display_name(),
                points_added,
                deep_per_asset
            );
            return Ok(deep_per_asset);
        }

        let err = anyhow!(
            "deep_price bootstrap via {} returned zero deep_per_asset",
            reference_pool.display_name()
        );
        tracing::warn!("Router: {}", err);
        last_err = Some(err);
    }

    Err(last_err.unwrap_or_else(|| anyhow!("deep_price bootstrap failed for all reference pools")))
}

fn seed_debug_pool_orderbook(state: &mut RouterEnvState, config: &DebugPoolCreateConfig) -> Result<()> {
    let deepbook_addr = AccountAddress::from_hex_literal(DEEPBOOK_PACKAGE)?;
    let sui_framework_addr = AccountAddress::from_hex_literal(SUI_FRAMEWORK_PACKAGE)?;

    let debug_tag = TypeTag::from_str(DEBUG_TYPE)?;
    let usdc_tag = TypeTag::from_str(USDC_TYPE)?;
    let deep_tag = TypeTag::from_str(DEEP_TYPE)?;
    let bm_tag = TypeTag::from_str(&format!(
        "{}::balance_manager::BalanceManager",
        DEEPBOOK_PACKAGE
    ))?;
    if config.whitelisted_pool || !config.pay_with_deep {
        tracing::info!(
            "Router: skipping deep_price bootstrap (whitelisted={}, pay_with_deep={})",
            config.whitelisted_pool,
            config.pay_with_deep
        );
    } else {
        let deep_per_asset = prime_debug_pool_deep_price(state)?;
        tracing::info!(
            "Router: primed debug deep_price using DEEP/USDC reference (deep_per_asset={})",
            deep_per_asset
        );
    }

    let original_sender = state.env.sender();
    let maker_sender = AccountAddress::from_hex_literal(DEBUG_POOL_MAKER_SENDER)?;
    state.env.set_sender(maker_sender);

    let seed_result = (|| -> Result<()> {
        let recipient = state.env.sender().to_vec();
        let place_seed_order = |state: &mut RouterEnvState,
                                client_order_id: u64,
                                price: u64,
                                quantity: u64,
                                is_bid: bool|
         -> Result<()> {
            let expiry_ms = state.clock_now_ms().saturating_add(DEBUG_ORDER_EXPIRY_TTL_MS);

            let inputs = vec![
                // 0) DBG/USDC pool (shared mutable)
                InputValue::Object(pool_shared_input(state, PoolId::DebugUsdc, true)?),
                // 1) DBG reserve coin
                InputValue::Object(reserve_coin_input(state, DEBUG_TYPE)?),
                // 2) USDC reserve coin
                InputValue::Object(reserve_coin_input(state, USDC_TYPE)?),
                // 3) DEEP reserve coin
                InputValue::Object(reserve_coin_input(state, DEEP_TYPE)?),
                // 4) client_order_id
                InputValue::Pure(bcs::to_bytes(&client_order_id)?),
                // 5) order_type = no_restriction
                InputValue::Pure(bcs::to_bytes(&0_u8)?),
                // 6) self_matching_option = allowed
                InputValue::Pure(bcs::to_bytes(&0_u8)?),
                // 7) price
                InputValue::Pure(bcs::to_bytes(&price)?),
                // 8) quantity
                InputValue::Pure(bcs::to_bytes(&quantity)?),
                // 9) is_bid
                InputValue::Pure(bcs::to_bytes(&is_bid)?),
                // 10) pay_with_deep
                InputValue::Pure(bcs::to_bytes(&config.pay_with_deep)?),
                // 11) expiry
                InputValue::Pure(bcs::to_bytes(&expiry_ms)?),
                // 12) clock
                InputValue::Object(state.next_clock_input()?),
                // 13) recipient to keep balance manager alive
                InputValue::Pure(recipient.clone()),
                // 14) DBG liquidity amount
                InputValue::Pure(bcs::to_bytes(&config.base_liquidity)?),
                // 15) USDC liquidity amount
                InputValue::Pure(bcs::to_bytes(&config.quote_liquidity)?),
                // 16) DEEP fee amount
                InputValue::Pure(bcs::to_bytes(&config.deep_fee_budget)?),
            ];

            let commands = vec![
                // 0) split DBG liquidity from reserve
                Command::MoveCall {
                    package: sui_framework_addr,
                    module: Identifier::new("coin")?,
                    function: Identifier::new("split")?,
                    type_args: vec![debug_tag.clone()],
                    args: vec![Argument::Input(1), Argument::Input(14)],
                },
                // 1) split USDC liquidity from reserve
                Command::MoveCall {
                    package: sui_framework_addr,
                    module: Identifier::new("coin")?,
                    function: Identifier::new("split")?,
                    type_args: vec![usdc_tag.clone()],
                    args: vec![Argument::Input(2), Argument::Input(15)],
                },
                // 2) split DEEP fee budget from reserve
                Command::MoveCall {
                    package: sui_framework_addr,
                    module: Identifier::new("coin")?,
                    function: Identifier::new("split")?,
                    type_args: vec![deep_tag.clone()],
                    args: vec![Argument::Input(3), Argument::Input(16)],
                },
                // 3) create balance manager
                Command::MoveCall {
                    package: deepbook_addr,
                    module: Identifier::new("balance_manager")?,
                    function: Identifier::new("new")?,
                    type_args: vec![],
                    args: vec![],
                },
                // 4) generate owner trade proof
                Command::MoveCall {
                    package: deepbook_addr,
                    module: Identifier::new("balance_manager")?,
                    function: Identifier::new("generate_proof_as_owner")?,
                    type_args: vec![],
                    args: vec![Argument::NestedResult(3, 0)],
                },
                // 5) deposit DBG
                Command::MoveCall {
                    package: deepbook_addr,
                    module: Identifier::new("balance_manager")?,
                    function: Identifier::new("deposit")?,
                    type_args: vec![debug_tag.clone()],
                    args: vec![Argument::NestedResult(3, 0), Argument::Result(0)],
                },
                // 6) deposit USDC
                Command::MoveCall {
                    package: deepbook_addr,
                    module: Identifier::new("balance_manager")?,
                    function: Identifier::new("deposit")?,
                    type_args: vec![usdc_tag.clone()],
                    args: vec![Argument::NestedResult(3, 0), Argument::Result(1)],
                },
                // 7) deposit DEEP
                Command::MoveCall {
                    package: deepbook_addr,
                    module: Identifier::new("balance_manager")?,
                    function: Identifier::new("deposit")?,
                    type_args: vec![deep_tag.clone()],
                    args: vec![Argument::NestedResult(3, 0), Argument::Result(2)],
                },
                // 8) place limit order
                Command::MoveCall {
                    package: deepbook_addr,
                    module: Identifier::new("pool")?,
                    function: Identifier::new("place_limit_order")?,
                    type_args: vec![debug_tag.clone(), usdc_tag.clone()],
                    args: vec![
                        Argument::Input(0),
                        Argument::NestedResult(3, 0),
                        Argument::NestedResult(4, 0),
                        Argument::Input(4),
                        Argument::Input(5),
                        Argument::Input(6),
                        Argument::Input(7),
                        Argument::Input(8),
                        Argument::Input(9),
                        Argument::Input(10),
                        Argument::Input(11),
                        Argument::Input(12),
                    ],
                },
                // 9) read order_info.order_id
                Command::MoveCall {
                    package: deepbook_addr,
                    module: Identifier::new("order_info")?,
                    function: Identifier::new("order_id")?,
                    type_args: vec![],
                    args: vec![Argument::NestedResult(8, 0)],
                },
                // 10) read order_info.price
                Command::MoveCall {
                    package: deepbook_addr,
                    module: Identifier::new("order_info")?,
                    function: Identifier::new("price")?,
                    type_args: vec![],
                    args: vec![Argument::NestedResult(8, 0)],
                },
                // 11) read order_info.original_quantity
                Command::MoveCall {
                    package: deepbook_addr,
                    module: Identifier::new("order_info")?,
                    function: Identifier::new("original_quantity")?,
                    type_args: vec![],
                    args: vec![Argument::NestedResult(8, 0)],
                },
                // 12) read order_info.executed_quantity
                Command::MoveCall {
                    package: deepbook_addr,
                    module: Identifier::new("order_info")?,
                    function: Identifier::new("executed_quantity")?,
                    type_args: vec![],
                    args: vec![Argument::NestedResult(8, 0)],
                },
                // 13) read order_info.cumulative_quote_quantity
                Command::MoveCall {
                    package: deepbook_addr,
                    module: Identifier::new("order_info")?,
                    function: Identifier::new("cumulative_quote_quantity")?,
                    type_args: vec![],
                    args: vec![Argument::NestedResult(8, 0)],
                },
                // 14) read order_info.status
                Command::MoveCall {
                    package: deepbook_addr,
                    module: Identifier::new("order_info")?,
                    function: Identifier::new("status")?,
                    type_args: vec![],
                    args: vec![Argument::NestedResult(8, 0)],
                },
                // 15) read order_info.order_inserted
                Command::MoveCall {
                    package: deepbook_addr,
                    module: Identifier::new("order_info")?,
                    function: Identifier::new("order_inserted")?,
                    type_args: vec![],
                    args: vec![Argument::NestedResult(8, 0)],
                },
                // 16) read pool vault balances after order placement.
                Command::MoveCall {
                    package: deepbook_addr,
                    module: Identifier::new("pool")?,
                    function: Identifier::new("vault_balances")?,
                    type_args: vec![debug_tag.clone(), usdc_tag.clone()],
                    args: vec![Argument::Input(0)],
                },
                // 17) transfer balance manager out so it persists.
                Command::MoveCall {
                    package: sui_framework_addr,
                    module: Identifier::new("transfer")?,
                    function: Identifier::new("public_transfer")?,
                    type_args: vec![bm_tag.clone()],
                    args: vec![Argument::NestedResult(3, 0), Argument::Input(13)],
                },
            ];

            let result = state.env.execute_ptb(inputs, commands);
            if !result.success {
                return Err(anyhow!(
                    "debug pool {} seed order failed: {}",
                    if is_bid { "bid" } else { "ask" },
                    result
                        .raw_error
                        .unwrap_or_else(|| "Unknown error".to_string())
                ));
            }
            let effects = result
                .effects
                .as_ref()
                .ok_or_else(|| anyhow!("Missing PTB effects for debug {} seed", if is_bid { "bid" } else { "ask" }))?;
            tracing::info!(
                "Router: debug {} seed effects mutated={}, created={}, dynamic_fields={}",
                if is_bid { "bid" } else { "ask" },
                effects.mutated.len(),
                effects.created.len(),
                effects.dynamic_field_entries.len()
            );
            for id in &effects.mutated {
                let type_hint = state
                    .env
                    .get_object(id)
                    .map(|obj| obj.type_tag.to_string())
                    .unwrap_or_else(|| "<missing>".to_string());
                let bytes_len = effects
                    .mutated_object_bytes
                    .get(id)
                    .map(|bytes| bytes.len())
                    .unwrap_or(0);
                tracing::info!(
                    "Router: debug {} seed mutated id={} type_hint={} bytes={}",
                    if is_bid { "bid" } else { "ask" },
                    id,
                    type_hint,
                    bytes_len
                );
            }
            for id in &effects.created {
                let type_hint = state
                    .env
                    .get_object(id)
                    .map(|obj| obj.type_tag.to_string())
                    .unwrap_or_else(|| "<missing>".to_string());
                let bytes_len = effects
                    .created_object_bytes
                    .get(id)
                    .map(|bytes| bytes.len())
                    .unwrap_or(0);
                tracing::info!(
                    "Router: debug {} seed created id={} type_hint={} bytes={}",
                    if is_bid { "bid" } else { "ask" },
                    id,
                    type_hint,
                    bytes_len
                );
            }
            let created_slice_fields: Vec<(
                AccountAddress,
                Option<AccountAddress>,
                Option<AccountAddress>,
                Option<u64>,
                bool,
            )> =
                effects
                    .object_changes
                    .iter()
                    .filter_map(|change| match change {
                        sui_sandbox_core::ptb::ObjectChange::Created {
                            id,
                            owner,
                            object_type: Some(type_tag),
                        } if type_tag.to_string().contains("big_vector::Slice") => {
                            let parent = parse_parent_from_owner_debug(owner);
                            let effect_parent = effects
                                .dynamic_field_entries
                                .iter()
                                .find_map(|((parent_id, child_id), _)| {
                                    (child_id == id).then_some(*parent_id)
                                });
                            let key = effects
                                .created_object_bytes
                                .get(id)
                                .and_then(|bytes| parse_dynamic_field_u64_name(bytes));
                            let present_in_effect_fields = effects
                                .dynamic_field_entries
                                .iter()
                                .any(|((_, child_id), _)| child_id == id);
                            Some((*id, parent, effect_parent, key, present_in_effect_fields))
                        }
                        _ => None,
                    })
                    .collect();
            if !created_slice_fields.is_empty() {
                tracing::info!(
                    "Router: debug {} seed created slice fields {:?}",
                    if is_bid { "bid" } else { "ask" },
                    created_slice_fields
                );
            }
            let placed_order_id =
                parse_u128_command_return(effects, 9, 0, "order_info.order_id")?;
            let order_price = parse_u64_command_return(effects, 10, 0, "order_info.price")?;
            let original_quantity =
                parse_u64_command_return(effects, 11, 0, "order_info.original_quantity")?;
            let executed_quantity =
                parse_u64_command_return(effects, 12, 0, "order_info.executed_quantity")?;
            let remaining_quantity = original_quantity.saturating_sub(executed_quantity);
            let cumulative_quote_quantity =
                parse_u64_command_return(effects, 13, 0, "order_info.cumulative_quote_quantity")?;
            let order_status = parse_u8_command_return(effects, 14, 0, "order_info.status")?;
            let order_inserted = parse_bool_command_return(effects, 15, 0, "order_info.inserted")?;
            let vault_base_after = parse_u64_command_return(effects, 16, 0, "vault_base_after")?;
            let vault_quote_after =
                parse_u64_command_return(effects, 16, 1, "vault_quote_after")?;
            let vault_deep_after = parse_u64_command_return(effects, 16, 2, "vault_deep_after")?;
            tracing::info!(
                "Router: debug {} seed order_info order_id={}, price={}, original_qty={}, executed_qty={}, cumulative_quote_qty={}, status={}, inserted={}, vault_after(base={}, quote={}, deep={})",
                if is_bid { "bid" } else { "ask" },
                placed_order_id,
                order_price,
                original_quantity,
                executed_quantity,
                cumulative_quote_quantity,
                order_status,
                order_inserted,
                vault_base_after,
                vault_quote_after,
                vault_deep_after
            );
            if let Some(pool_entry) = state.pool_cache.get(&PoolId::DebugUsdc) {
                if let Some(pool_obj) = state.env.get_object(&pool_entry.pool_addr) {
                    if pool_obj.bcs_bytes.len() >= 72 {
                        let mut inner_parent_bytes = [0u8; AccountAddress::LENGTH];
                        inner_parent_bytes.copy_from_slice(&pool_obj.bcs_bytes[32..64]);
                        let inner_parent = AccountAddress::new(inner_parent_bytes);
                        let mut inner_version_bytes = [0u8; 8];
                        inner_version_bytes.copy_from_slice(&pool_obj.bcs_bytes[64..72]);
                        let inner_version = u64::from_le_bytes(inner_version_bytes);
                        let matching_inner_fields: Vec<(AccountAddress, String, Option<u64>)> =
                            effects
                                .dynamic_field_entries
                                .iter()
                                .filter(|((parent_id, _), (type_tag, _))| {
                                    *parent_id == inner_parent
                                        && type_tag
                                            .to_string()
                                            .contains("::pool::PoolInner<")
                                })
                                .map(|((_, child_id), (type_tag, bytes))| {
                                    (
                                        *child_id,
                                        type_tag.to_string(),
                                        parse_dynamic_field_u64_name(bytes),
                                    )
                                })
                                .collect();
                        if !matching_inner_fields.is_empty() {
                            tracing::info!(
                                "Router: debug {} seed inner parent {} wrapper_version={} fields_in_effects={:?}",
                                if is_bid { "bid" } else { "ask" },
                                inner_parent,
                                inner_version,
                                matching_inner_fields
                            );
                        }
                    }
                }
            }
            sync_dynamic_field_entries(state, effects);
            for (_child_id, _owner_parent, effect_parent, key, _present_in_effect_fields) in
                &created_slice_fields
            {
                let (Some(parent), Some(slice_key)) = (*effect_parent, *key) else {
                    continue;
                };
                if let Err(e) = patch_pool_big_vector_header_from_created_slice(
                    state,
                    PoolId::DebugUsdc,
                    parent,
                    slice_key,
                ) {
                    tracing::warn!(
                        "Router: failed patching debug BigVector header from slice parent={} key={}: {}",
                        parent,
                        slice_key,
                        e
                    );
                }
            }
            if order_inserted && remaining_quantity > 0 {
                let (add_base, add_quote) = if is_bid {
                    (0_u64, scaled_mul_floor(remaining_quantity, order_price))
                } else {
                    (remaining_quantity, 0_u64)
                };
                if let Err(e) =
                    patch_pool_vault_tail_for_seed(state, PoolId::DebugUsdc, add_base, add_quote, 0)
                {
                    tracing::warn!(
                        "Router: failed patching debug vault tail (is_bid={}, add_base={}, add_quote={}): {}",
                        is_bid,
                        add_base,
                        add_quote,
                        e
                    );
                }
            }
            if !created_slice_fields.is_empty() {
                let mut registered = Vec::new();
                for (child_id, owner_parent, effect_parent, key, _present_in_effect_fields) in
                    &created_slice_fields
                {
                    let exists_via_owner = owner_parent
                        .and_then(|parent_id| state.env.get_dynamic_field(parent_id, *child_id))
                        .is_some();
                    let exists_via_effect = effect_parent
                        .and_then(|parent_id| state.env.get_dynamic_field(parent_id, *child_id))
                        .is_some();
                    registered.push((
                        *child_id,
                        *owner_parent,
                        *effect_parent,
                        *key,
                        exists_via_owner,
                        exists_via_effect,
                    ));
                }
                tracing::info!(
                    "Router: debug {} seed slice registration after sync {:?}",
                    if is_bid { "bid" } else { "ask" },
                    registered
                );
            }
            if order_inserted {
                if let Err(e) = log_debug_order_lookup(
                    state,
                    if is_bid {
                        "post-bid-seed"
                    } else {
                        "post-ask-seed"
                    },
                    placed_order_id,
                ) {
                    tracing::warn!("Router: debug get_order lookup failed: {}", e);
                }
            }
            Ok(())
        };

        place_seed_order(state, 1, config.ask_price, config.ask_quantity, false)?;
        log_debug_pool_snapshot(state, "after-ask-seed")?;
        place_seed_order(state, 2, config.bid_price, config.bid_quantity, true)?;
        log_debug_pool_snapshot(state, "post-seed")?;

        Ok(())
    })();

    state.env.set_sender(original_sender);
    seed_result
}

/// Execute a two-hop quote via the MoveVM router contract
fn execute_two_hop_quote(
    state: &mut RouterEnvState,
    from_pool: PoolId,
    to_pool: PoolId,
    input_amount: u64,
) -> Result<TwoHopQuote> {
    if !state.router_deployed {
        return Err(anyhow!(
            "Router contract not deployed (two-hop quotes unavailable)"
        ));
    }

    // Determine type args: A (base of from_pool), Q (USDC), B (base of to_pool)
    let (a_type, q_type, b_type) = resolve_two_hop_types(from_pool, to_pool)?;

    let a_tag = TypeTag::from_str(a_type)?;
    let q_tag = TypeTag::from_str(q_type)?;
    let b_tag = TypeTag::from_str(b_type)?;

    let router_addr = AccountAddress::from_hex_literal(ROUTER_PACKAGE_ADDR)?;
    let clock_input = state.next_clock_input()?;

    let inputs = vec![
        // Input 0: Pool<A, Q> (shared, immutable ref)
        InputValue::Object(pool_shared_input(state, from_pool, false)?),
        // Input 1: Pool<B, Q> (shared, immutable ref)
        InputValue::Object(pool_shared_input(state, to_pool, false)?),
        // Input 2: input_amount (pure u64)
        InputValue::Pure(bcs::to_bytes(&input_amount)?),
        // Input 3: Clock at 0x6 (shared, immutable ref)
        InputValue::Object(clock_input),
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
            result
                .raw_error
                .unwrap_or_else(|| "Unknown error".to_string())
        ));
    }

    // Parse return values: (u64, u64) = (final_output, intermediate_amount)
    let return_values = result
        .effects
        .as_ref()
        .and_then(|effects| effects.return_values.first())
        .ok_or_else(|| anyhow!("No return values from quote_two_hop"))?;

    // First return value: base_out (u64)
    let final_output = parse_u64_return(return_values, 0, "final_output")?;

    // Second return value: quote_out (u64)
    let intermediate_amount = parse_u64_return(return_values, 1, "intermediate_amount")?;

    Ok(TwoHopQuote {
        final_output,
        intermediate_amount,
    })
}

fn execute_vm_faucet(
    state: &mut RouterEnvState,
    coin_type: &str,
    amount: u64,
) -> Result<VmFaucetResult> {
    let sui_framework_addr = AccountAddress::from_hex_literal(SUI_FRAMEWORK_PACKAGE)?;
    let coin_tag = TypeTag::from_str(coin_type)?;
    let coin_obj_tag = TypeTag::from_str(&format!("0x2::coin::Coin<{}>", coin_type))?;
    let recipient = state.env.sender().to_vec();

    let inputs = vec![
        InputValue::Object(reserve_coin_input(state, coin_type)?),
        InputValue::Pure(bcs::to_bytes(&amount)?),
        InputValue::Pure(recipient),
    ];

    let commands = vec![
        // Split faucet amount from a persistent VM reserve coin.
        Command::MoveCall {
            package: sui_framework_addr,
            module: Identifier::new("coin")?,
            function: Identifier::new("split")?,
            type_args: vec![coin_tag.clone()],
            args: vec![Argument::Input(0), Argument::Input(1)],
        },
        // Read returned coin value for verification.
        Command::MoveCall {
            package: sui_framework_addr,
            module: Identifier::new("coin")?,
            function: Identifier::new("value")?,
            type_args: vec![coin_tag],
            args: vec![Argument::Result(0)],
        },
        // Transfer faucet coin to sender in VM so object lifecycle is legitimate.
        Command::MoveCall {
            package: sui_framework_addr,
            module: Identifier::new("transfer")?,
            function: Identifier::new("public_transfer")?,
            type_args: vec![coin_obj_tag],
            args: vec![Argument::Result(0), Argument::Input(2)],
        },
    ];

    let result = state.env.execute_ptb(inputs, commands);
    if !result.success {
        return Err(anyhow!(
            "vm faucet split/transfer failed for {}: {}",
            coin_type,
            result
                .raw_error
                .unwrap_or_else(|| "Unknown error".to_string())
        ));
    }

    let effects = result
        .effects
        .as_ref()
        .ok_or_else(|| anyhow!("Missing PTB effects for vm faucet"))?;
    let minted_amount = parse_u64_command_return(effects, 1, 0, "faucet_amount")?;
    if minted_amount != amount {
        return Err(anyhow!(
            "VM faucet amount mismatch for {}: requested {}, minted {}",
            coin_type,
            amount,
            minted_amount
        ));
    }

    Ok(VmFaucetResult {
        amount: minted_amount,
        gas_used: effects.gas_used,
        created_objects: effects.created.iter().map(|id| id.to_string()).collect(),
        events: collect_swap_events(effects),
    })
}

fn execute_single_hop_swap(
    state: &mut RouterEnvState,
    pool_id: PoolId,
    input_amount: u64,
    deep_amount: u64,
    is_sell_base: bool,
) -> Result<SingleHopSwapResult> {
    let (base_type, quote_type) = pool_types(pool_id);
    let base_tag = TypeTag::from_str(base_type)?;
    let quote_tag = TypeTag::from_str(quote_type)?;
    let (input_coin_type, output_coin_type, swap_fn, output_idx, refund_idx) = if is_sell_base {
        (
            base_type,
            quote_type,
            "swap_exact_base_for_quote",
            1usize, // quote_out
            0usize, // base_refund
        )
    } else {
        (
            quote_type,
            base_type,
            "swap_exact_quote_for_base",
            0usize, // base_out
            1usize, // quote_refund
        )
    };
    let input_coin_tag = TypeTag::from_str(input_coin_type)?;
    let output_coin_tag = TypeTag::from_str(output_coin_type)?;
    let output_coin_obj_tag =
        TypeTag::from_str(&format!("0x2::coin::Coin<{}>", output_coin_type))?;

    let deepbook_addr = AccountAddress::from_hex_literal(DEEPBOOK_PACKAGE)?;
    let sui_framework_addr = AccountAddress::from_hex_literal(SUI_FRAMEWORK_PACKAGE)?;
    let recipient = state.env.sender().to_vec();
    let min_out: u64 = 0;

    let inputs = vec![
        InputValue::Object(pool_shared_input(state, pool_id, true)?),
        InputValue::Object(reserve_coin_input(state, input_coin_type)?),
        InputValue::Object(reserve_coin_input(state, DEEP_TYPE)?),
        InputValue::Pure(bcs::to_bytes(&input_amount)?),
        InputValue::Pure(bcs::to_bytes(&deep_amount)?),
        InputValue::Pure(bcs::to_bytes(&min_out)?),
        InputValue::Object(state.next_clock_input()?),
        InputValue::Pure(recipient),
    ];

    let commands = vec![
        // Create input coin via VM split from reserve.
        Command::MoveCall {
            package: sui_framework_addr,
            module: Identifier::new("coin")?,
            function: Identifier::new("split")?,
            type_args: vec![input_coin_tag.clone()],
            args: vec![Argument::Input(1), Argument::Input(3)],
        },
        // Create DEEP fee coin via VM split from reserve.
        Command::MoveCall {
            package: sui_framework_addr,
            module: Identifier::new("coin")?,
            function: Identifier::new("split")?,
            type_args: vec![TypeTag::from_str(DEEP_TYPE)?],
            args: vec![Argument::Input(2), Argument::Input(4)],
        },
        // Execute the actual swap in MoveVM.
        Command::MoveCall {
            package: deepbook_addr,
            module: Identifier::new("pool")?,
            function: Identifier::new(swap_fn)?,
            type_args: vec![base_tag.clone(), quote_tag.clone()],
            args: vec![
                Argument::Input(0), // pool
                Argument::Result(0), // input coin
                Argument::Result(1), // deep coin
                Argument::Input(5),  // min out
                Argument::Input(6),  // clock
            ],
        },
        // Extract output amount from returned coin.
        Command::MoveCall {
            package: sui_framework_addr,
            module: Identifier::new("coin")?,
            function: Identifier::new("value")?,
            type_args: vec![output_coin_tag],
            args: vec![Argument::NestedResult(2, output_idx as u16)],
        },
        // Extract unspent input amount from returned input coin.
        Command::MoveCall {
            package: sui_framework_addr,
            module: Identifier::new("coin")?,
            function: Identifier::new("value")?,
            type_args: vec![input_coin_tag.clone()],
            args: vec![Argument::NestedResult(2, refund_idx as u16)],
        },
        // Extract any DEEP refund.
        Command::MoveCall {
            package: sui_framework_addr,
            module: Identifier::new("coin")?,
            function: Identifier::new("value")?,
            type_args: vec![TypeTag::from_str(DEEP_TYPE)?],
            args: vec![Argument::NestedResult(2, 2)],
        },
        // Join input refund back into reserve.
        Command::MoveCall {
            package: sui_framework_addr,
            module: Identifier::new("coin")?,
            function: Identifier::new("join")?,
            type_args: vec![input_coin_tag],
            args: vec![Argument::Input(1), Argument::NestedResult(2, refund_idx as u16)],
        },
        // Join DEEP refund back into reserve.
        Command::MoveCall {
            package: sui_framework_addr,
            module: Identifier::new("coin")?,
            function: Identifier::new("join")?,
            type_args: vec![TypeTag::from_str(DEEP_TYPE)?],
            args: vec![Argument::Input(2), Argument::NestedResult(2, 2)],
        },
        // Transfer output coin so lifecycle is fully VM-driven.
        Command::MoveCall {
            package: sui_framework_addr,
            module: Identifier::new("transfer")?,
            function: Identifier::new("public_transfer")?,
            type_args: vec![output_coin_obj_tag],
            args: vec![Argument::NestedResult(2, output_idx as u16), Argument::Input(7)],
        },
    ];

    let result = state.env.execute_ptb(inputs, commands);
    if !result.success {
        return Err(anyhow!(
            "single-hop swap via pool::{} failed for {}: {}",
            swap_fn,
            pool_id.display_name(),
            result
                .raw_error
                .unwrap_or_else(|| "Unknown error".to_string())
        ));
    }

    let effects = result
        .effects
        .as_ref()
        .ok_or_else(|| anyhow!("Missing PTB effects for single-hop swap"))?;

    let output_amount = parse_u64_command_return(effects, 3, 0, "output_amount")?;
    let input_refund = parse_u64_command_return(effects, 4, 0, "input_refund")?;
    let deep_refund = parse_u64_command_return(effects, 5, 0, "deep_refund")?;
    if pool_id == PoolId::DebugUsdc {
        tracing::info!(
            "Router: debug single-hop swap {} output={}, input_refund={}, deep_refund={}, input={}, deep_in={}",
            swap_fn,
            output_amount,
            input_refund,
            deep_refund,
            input_amount,
            deep_amount
        );
    }

    Ok(SingleHopSwapResult {
        output_amount,
        input_refund,
        deep_refund,
        gas_used: effects.gas_used,
        events: collect_swap_events(effects),
    })
}

fn execute_two_hop_swap(
    state: &mut RouterEnvState,
    from_pool: PoolId,
    to_pool: PoolId,
    input_amount: u64,
    deep_amount: u64,
) -> Result<TwoHopSwapResult> {
    let (a_type, q_type, b_type) = resolve_two_hop_types(from_pool, to_pool)?;
    let a_tag = TypeTag::from_str(a_type)?;
    let q_tag = TypeTag::from_str(q_type)?;
    let b_tag = TypeTag::from_str(b_type)?;
    let b_coin_obj_tag = TypeTag::from_str(&format!("0x2::coin::Coin<{}>", b_type))?;

    let deepbook_addr = AccountAddress::from_hex_literal(DEEPBOOK_PACKAGE)?;
    let sui_framework_addr = AccountAddress::from_hex_literal(SUI_FRAMEWORK_PACKAGE)?;
    let recipient = state.env.sender().to_vec();
    let min_out: u64 = 0;

    let inputs = vec![
        InputValue::Object(pool_shared_input(state, from_pool, true)?),
        InputValue::Object(pool_shared_input(state, to_pool, true)?),
        InputValue::Object(reserve_coin_input(state, a_type)?),
        InputValue::Object(reserve_coin_input(state, q_type)?),
        InputValue::Object(reserve_coin_input(state, DEEP_TYPE)?),
        InputValue::Pure(bcs::to_bytes(&input_amount)?),
        InputValue::Pure(bcs::to_bytes(&deep_amount)?),
        InputValue::Pure(bcs::to_bytes(&min_out)?),
        InputValue::Object(state.next_clock_input()?),
        InputValue::Pure(recipient),
    ];

    let commands = vec![
        // Create A input coin from reserve.
        Command::MoveCall {
            package: sui_framework_addr,
            module: Identifier::new("coin")?,
            function: Identifier::new("split")?,
            type_args: vec![a_tag.clone()],
            args: vec![Argument::Input(2), Argument::Input(5)],
        },
        // Create DEEP fee coin from reserve.
        Command::MoveCall {
            package: sui_framework_addr,
            module: Identifier::new("coin")?,
            function: Identifier::new("split")?,
            type_args: vec![TypeTag::from_str(DEEP_TYPE)?],
            args: vec![Argument::Input(4), Argument::Input(6)],
        },
        // Leg 1: A -> USDC
        Command::MoveCall {
            package: deepbook_addr,
            module: Identifier::new("pool")?,
            function: Identifier::new("swap_exact_base_for_quote")?,
            type_args: vec![a_tag.clone(), q_tag.clone()],
            args: vec![
                Argument::Input(0), // first pool
                Argument::Result(0), // input coin A
                Argument::Result(1), // deep coin
                Argument::Input(7),  // min out
                Argument::Input(8),  // clock
            ],
        },
        // Capture intermediate USDC from leg 1.
        Command::MoveCall {
            package: sui_framework_addr,
            module: Identifier::new("coin")?,
            function: Identifier::new("value")?,
            type_args: vec![q_tag.clone()],
            args: vec![Argument::NestedResult(2, 1)],
        },
        // Leg 2: USDC -> B
        Command::MoveCall {
            package: deepbook_addr,
            module: Identifier::new("pool")?,
            function: Identifier::new("swap_exact_quote_for_base")?,
            type_args: vec![b_tag.clone(), q_tag.clone()],
            args: vec![
                Argument::Input(1),           // second pool
                Argument::NestedResult(2, 1), // intermediate quote coin
                Argument::NestedResult(2, 2), // deep coin from leg 1
                Argument::Input(7),           // min out
                Argument::Input(8),           // clock
            ],
        },
        // Extract B output.
        Command::MoveCall {
            package: sui_framework_addr,
            module: Identifier::new("coin")?,
            function: Identifier::new("value")?,
            type_args: vec![b_tag],
            args: vec![Argument::NestedResult(4, 0)],
        },
        // Extract A refund from leg 1.
        Command::MoveCall {
            package: sui_framework_addr,
            module: Identifier::new("coin")?,
            function: Identifier::new("value")?,
            type_args: vec![a_tag.clone()],
            args: vec![Argument::NestedResult(2, 0)],
        },
        // Extract quote refund from leg 2.
        Command::MoveCall {
            package: sui_framework_addr,
            module: Identifier::new("coin")?,
            function: Identifier::new("value")?,
            type_args: vec![q_tag],
            args: vec![Argument::NestedResult(4, 1)],
        },
        // Extract DEEP refund.
        Command::MoveCall {
            package: sui_framework_addr,
            module: Identifier::new("coin")?,
            function: Identifier::new("value")?,
            type_args: vec![TypeTag::from_str(DEEP_TYPE)?],
            args: vec![Argument::NestedResult(4, 2)],
        },
        // Join A refund back into reserve.
        Command::MoveCall {
            package: sui_framework_addr,
            module: Identifier::new("coin")?,
            function: Identifier::new("join")?,
            type_args: vec![a_tag],
            args: vec![Argument::Input(2), Argument::NestedResult(2, 0)],
        },
        // Join USDC refund from leg 2 back into reserve.
        Command::MoveCall {
            package: sui_framework_addr,
            module: Identifier::new("coin")?,
            function: Identifier::new("join")?,
            type_args: vec![TypeTag::from_str(q_type)?],
            args: vec![Argument::Input(3), Argument::NestedResult(4, 1)],
        },
        // Join DEEP refund from leg 2 back into reserve.
        Command::MoveCall {
            package: sui_framework_addr,
            module: Identifier::new("coin")?,
            function: Identifier::new("join")?,
            type_args: vec![TypeTag::from_str(DEEP_TYPE)?],
            args: vec![Argument::Input(4), Argument::NestedResult(4, 2)],
        },
        // Transfer B output so lifecycle is fully VM-driven.
        Command::MoveCall {
            package: sui_framework_addr,
            module: Identifier::new("transfer")?,
            function: Identifier::new("public_transfer")?,
            type_args: vec![b_coin_obj_tag],
            args: vec![Argument::NestedResult(4, 0), Argument::Input(9)],
        },
    ];

    let result = state.env.execute_ptb(inputs, commands);
    if !result.success {
        // Some debug-pool routes abort in the atomic two-hop PTB. Keep execution
        // VM-native by falling back to two sequential single-hop VM swaps.
        if from_pool == PoolId::DebugUsdc || to_pool == PoolId::DebugUsdc {
            tracing::warn!(
                "Router: two-hop atomic PTB failed for {} -> {}. Falling back to sequential VM hops.",
                from_pool.display_name(),
                to_pool.display_name()
            );
            return execute_two_hop_swap_sequential_vm(
                state,
                from_pool,
                to_pool,
                input_amount,
                deep_amount,
            );
        }
        return Err(anyhow!(
            "two-hop swap execution failed ({} -> {}): {}",
            from_pool.display_name(),
            to_pool.display_name(),
            result
                .raw_error
                .unwrap_or_else(|| "Unknown error".to_string())
        ));
    }

    let effects = result
        .effects
        .as_ref()
        .ok_or_else(|| anyhow!("Missing PTB effects for two-hop swap"))?;

    let intermediate_amount = parse_u64_command_return(effects, 3, 0, "intermediate_amount")?;
    let output_amount = parse_u64_command_return(effects, 5, 0, "output_amount")?;
    let input_refund = parse_u64_command_return(effects, 6, 0, "input_refund")?;
    let quote_refund = parse_u64_command_return(effects, 7, 0, "quote_refund")?;
    let deep_refund = parse_u64_command_return(effects, 8, 0, "deep_refund")?;

    Ok(TwoHopSwapResult {
        output_amount,
        intermediate_amount,
        input_refund,
        quote_refund,
        deep_refund,
        gas_used: effects.gas_used,
        events: collect_swap_events(effects),
    })
}

fn execute_two_hop_swap_sequential_vm(
    state: &mut RouterEnvState,
    from_pool: PoolId,
    to_pool: PoolId,
    input_amount: u64,
    deep_amount: u64,
) -> Result<TwoHopSwapResult> {
    // Hop 1: A -> USDC (sell base)
    let hop1 = execute_single_hop_swap(state, from_pool, input_amount, deep_amount, true)?;
    // Hop 2: USDC -> B (sell quote/base=false), using leftover DEEP from hop 1.
    let hop2 = execute_single_hop_swap(
        state,
        to_pool,
        hop1.output_amount,
        hop1.deep_refund,
        false,
    )?;

    let mut events = hop1.events;
    events.extend(hop2.events);

    Ok(TwoHopSwapResult {
        output_amount: hop2.output_amount,
        intermediate_amount: hop1.output_amount,
        input_refund: hop1.input_refund,
        quote_refund: hop2.input_refund,
        deep_refund: hop2.deep_refund,
        gas_used: hop1.gas_used.saturating_add(hop2.gas_used),
        events,
    })
}

/// Resolve type arguments for a two-hop swap: A -> USDC -> B
fn resolve_two_hop_types(
    from_pool: PoolId,
    to_pool: PoolId,
) -> Result<(&'static str, &'static str, &'static str)> {
    let a_type = match from_pool {
        PoolId::SuiUsdc => SUI_TYPE,
        PoolId::WalUsdc => WAL_TYPE,
        PoolId::DeepUsdc => DEEP_TYPE,
        PoolId::DebugUsdc => DEBUG_TYPE,
    };

    let b_type = match to_pool {
        PoolId::SuiUsdc => SUI_TYPE,
        PoolId::WalUsdc => WAL_TYPE,
        PoolId::DeepUsdc => DEEP_TYPE,
        PoolId::DebugUsdc => DEBUG_TYPE,
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
        .map_err(|e| {
            anyhow!(
                "Router BCS conversion failed for object {} (type: {}): {}",
                obj.object_id,
                obj.object_type,
                e
            )
        })?;

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
        .map_err(|e| {
            anyhow!(
                "Router BCS conversion failed for dynamic field {} (type: {}): {}",
                obj.object_id,
                corrected_type,
                e
            )
        })?;

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

fn synthesize_account_dynamic_fields_for_router(
    env: &mut SimulationEnvironment,
    bcs_converter: &mut JsonToBcsConverter,
    loader: &StateLoader,
) -> Result<usize> {
    let Some(accounts_table_id) = extract_accounts_table_id(loader) else {
        tracing::warn!(
            "Router: {} missing state.accounts table; skipping account-field synthesis",
            loader.config().pool_id.display_name()
        );
        return Ok(0);
    };
    let accounts_table_addr = AccountAddress::from_hex_literal(&accounts_table_id)?;

    let account_field_type = format!(
        "0x2::dynamic_field::Field<{}, {}::account::Account>",
        OBJECT_ID_TYPE, DEEPBOOK_PACKAGE
    );
    let account_field_tag = SimulationEnvironment::parse_type_string(&account_field_type)
        .ok_or_else(|| anyhow!("Failed to parse type: {}", account_field_type))?;
    let object_id_tag = TypeTag::from_str(OBJECT_ID_TYPE)?;

    let mut existing_child_ids = HashSet::new();
    for obj in loader.all_objects() {
        if obj.owner_address.as_deref() == Some(accounts_table_id.as_str())
            && obj.object_type.contains("dynamic_field::Field")
        {
            if let Ok(child_id) = AccountAddress::from_hex_literal(&obj.object_id) {
                existing_child_ids.insert(child_id);
            }
        }
    }

    let mut order_ids_by_balance_manager: HashMap<String, HashSet<u128>> = HashMap::new();
    for obj in loader.all_objects() {
        if !(obj.object_type.contains("big_vector::Slice")
            && obj.object_type.contains("order::Order"))
        {
            continue;
        }

        let Some(vals) = obj
            .object_json
            .get("value")
            .and_then(|value| value.get("vals"))
            .and_then(|vals| vals.as_array())
        else {
            continue;
        };

        for order in vals {
            let Some(balance_manager_id) = order.get("balance_manager_id").and_then(|v| v.as_str())
            else {
                continue;
            };
            let Some(order_id_str) = order.get("order_id").and_then(|v| v.as_str()) else {
                continue;
            };
            let Ok(order_id) = order_id_str.parse::<u128>() else {
                continue;
            };

            order_ids_by_balance_manager
                .entry(balance_manager_id.to_string())
                .or_default()
                .insert(order_id);
        }
    }

    let mut synthesized = 0usize;
    for (balance_manager_id, mut order_ids) in order_ids_by_balance_manager {
        let manager_addr = match AccountAddress::from_hex_literal(&balance_manager_id) {
            Ok(addr) => addr,
            Err(err) => {
                tracing::warn!(
                    "Router: skipping malformed balance_manager_id {}: {}",
                    balance_manager_id,
                    err
                );
                continue;
            }
        };

        let key_bytes = bcs::to_bytes(&manager_addr)
            .map_err(|e| anyhow!("Failed to encode synthetic account key: {}", e))?;

        let child_id = derive_dynamic_field_id(accounts_table_addr, &object_id_tag, &key_bytes)
            .map_err(|e| anyhow!("Failed to derive account dynamic field ID: {}", e))?;

        if existing_child_ids.contains(&child_id) {
            continue;
        }

        let mut open_orders: Vec<u128> = order_ids.drain().collect();
        open_orders.sort_unstable();

        let field_json = json!({
            "id": { "id": child_id.to_hex_literal() },
            "name": { "id": manager_addr.to_hex_literal() },
            "value": {
                "epoch": "0",
                "open_orders": {
                    "contents": open_orders
                        .into_iter()
                        .map(|order_id| serde_json::Value::String(order_id.to_string()))
                        .collect::<Vec<_>>()
                },
                "taker_volume": "0",
                "maker_volume": "0",
                "active_stake": "0",
                "inactive_stake": "0",
                "created_proposal": false,
                "voted_proposal": serde_json::Value::Null,
                "unclaimed_rebates": { "base": "0", "quote": "0", "deep": "0" },
                "settled_balances": { "base": "0", "quote": "0", "deep": "0" },
                "owed_balances": { "base": "0", "quote": "0", "deep": "0" }
            }
        });

        let field_bytes = bcs_converter
            .convert(&account_field_type, &field_json)
            .map_err(|e| {
                anyhow!(
                    "Failed to encode synthetic account dynamic field for {}: {}",
                    manager_addr,
                    e
                )
            })?;

        env.set_dynamic_field(
            accounts_table_addr,
            child_id,
            account_field_tag.clone(),
            field_bytes,
        );
        existing_child_ids.insert(child_id);
        synthesized += 1;
    }

    Ok(synthesized)
}

fn extract_accounts_table_id(loader: &StateLoader) -> Option<String> {
    loader.all_objects().find_map(|obj| {
        if !obj.object_type.contains("pool::PoolInner") {
            return None;
        }

        obj.object_json
            .get("value")
            .and_then(|value| value.get("state"))
            .and_then(|state| state.get("accounts"))
            .and_then(|accounts| accounts.get("id"))
            .and_then(|id| id.get("id"))
            .and_then(|id| id.as_str())
            .map(|id| id.to_string())
    })
}

fn extract_pool_epoch(loader: &StateLoader) -> Option<u64> {
    loader.all_objects().find_map(|obj| {
        if !obj.object_type.contains("pool::PoolInner") {
            return None;
        }

        obj.object_json
            .get("value")
            .and_then(|value| value.get("state"))
            .and_then(|state| state.get("history"))
            .and_then(|history| history.get("epoch"))
            .and_then(|epoch| epoch.as_str())
            .and_then(|epoch| epoch.parse::<u64>().ok())
    })
}

#[derive(Debug, Clone, Copy)]
struct TradeParamsSnapshot {
    taker_fee: u64,
    maker_fee: u64,
    stake_required: u64,
}

#[derive(Debug, Clone)]
struct HistorySynthesisContext {
    table_id: String,
    history_epoch: u64,
    trade_params: TradeParamsSnapshot,
}

fn synthesize_history_volume_fields_for_router(
    env: &mut SimulationEnvironment,
    bcs_converter: &mut JsonToBcsConverter,
    loader: &StateLoader,
) -> Result<usize> {
    let Some(ctx) = extract_history_synthesis_context(loader) else {
        return Ok(0);
    };

    let table_addr = AccountAddress::from_hex_literal(&ctx.table_id)?;
    let field_type = format!(
        "0x2::dynamic_field::Field<u64, {}::history::Volumes>",
        DEEPBOOK_PACKAGE
    );
    let field_tag = SimulationEnvironment::parse_type_string(&field_type)
        .ok_or_else(|| anyhow!("Failed to parse type: {}", field_type))?;
    let key_type = TypeTag::U64;

    let mut existing_child_ids = HashSet::new();
    for obj in loader.all_objects() {
        if obj.owner_address.as_deref() == Some(ctx.table_id.as_str())
            && obj.object_type.contains("dynamic_field::Field")
        {
            if let Ok(child_id) = AccountAddress::from_hex_literal(&obj.object_id) {
                existing_child_ids.insert(child_id);
            }
        }
    }

    let mut epochs = HashSet::new();
    epochs.insert(ctx.history_epoch);

    for obj in loader.all_objects() {
        if !(obj.object_type.contains("big_vector::Slice")
            && obj.object_type.contains("order::Order"))
        {
            continue;
        }

        let Some(vals) = obj
            .object_json
            .get("value")
            .and_then(|value| value.get("vals"))
            .and_then(|vals| vals.as_array())
        else {
            continue;
        };

        for order in vals {
            let Some(epoch_str) = order.get("epoch").and_then(|v| v.as_str()) else {
                continue;
            };
            if let Ok(epoch) = epoch_str.parse::<u64>() {
                epochs.insert(epoch);
            }
        }
    }

    let mut epochs_sorted: Vec<u64> = epochs.into_iter().collect();
    epochs_sorted.sort_unstable();

    let mut synthesized = 0usize;
    for epoch in epochs_sorted {
        let key_bytes = bcs::to_bytes(&epoch)
            .map_err(|e| anyhow!("Failed to encode history epoch key: {}", e))?;
        let child_id = derive_dynamic_field_id(table_addr, &key_type, &key_bytes)
            .map_err(|e| anyhow!("Failed to derive history dynamic field ID: {}", e))?;

        if existing_child_ids.contains(&child_id) {
            continue;
        }

        let field_json = json!({
            "id": { "id": child_id.to_hex_literal() },
            "name": epoch.to_string(),
            "value": {
                "total_volume": "0",
                "total_staked_volume": "0",
                "total_fees_collected": { "base": "0", "quote": "0", "deep": "0" },
                "historic_median": "0",
                "trade_params": {
                    "taker_fee": ctx.trade_params.taker_fee.to_string(),
                    "maker_fee": ctx.trade_params.maker_fee.to_string(),
                    "stake_required": ctx.trade_params.stake_required.to_string()
                }
            }
        });

        let field_bytes = bcs_converter
            .convert(&field_type, &field_json)
            .map_err(|e| {
                anyhow!(
                    "Failed to encode synthetic history dynamic field for epoch {}: {}",
                    epoch,
                    e
                )
            })?;

        env.set_dynamic_field(table_addr, child_id, field_tag.clone(), field_bytes);
        existing_child_ids.insert(child_id);
        synthesized += 1;
    }

    Ok(synthesized)
}

fn extract_history_synthesis_context(loader: &StateLoader) -> Option<HistorySynthesisContext> {
    loader.all_objects().find_map(|obj| {
        if !obj.object_type.contains("pool::PoolInner") {
            return None;
        }

        let value = obj.object_json.get("value")?;
        let state = value.get("state")?;
        let history = state.get("history")?;
        let governance = state.get("governance")?;
        let trade_params = governance.get("trade_params")?;

        let table_id = history
            .get("historic_volumes")
            .and_then(|hv| hv.get("id"))
            .and_then(|id| id.get("id"))
            .and_then(|id| id.as_str())?
            .to_string();
        let history_epoch = history
            .get("epoch")
            .and_then(|epoch| epoch.as_str())
            .and_then(|epoch| epoch.parse::<u64>().ok())?;
        let taker_fee = trade_params
            .get("taker_fee")
            .and_then(|v| v.as_str())
            .and_then(|v| v.parse::<u64>().ok())?;
        let maker_fee = trade_params
            .get("maker_fee")
            .and_then(|v| v.as_str())
            .and_then(|v| v.parse::<u64>().ok())?;
        let stake_required = trade_params
            .get("stake_required")
            .and_then(|v| v.as_str())
            .and_then(|v| v.parse::<u64>().ok())?;

        Some(HistorySynthesisContext {
            table_id,
            history_epoch,
            trade_params: TradeParamsSnapshot {
                taker_fee,
                maker_fee,
                stake_required,
            },
        })
    })
}
