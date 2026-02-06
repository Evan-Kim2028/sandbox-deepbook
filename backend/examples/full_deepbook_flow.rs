//! Full DeepBook local flow runner (no HTTP backend server).
//!
//! This example validates end-to-end functionality purely in-process:
//! 1. Load Snowflake JSONL state for all supported pools
//! 2. Build MoveVM orderbooks via deepbook::order_query::iter_orders
//! 3. Create a trading session
//! 4. Run direct swap flow (SUI -> USDC)
//! 5. Run two-hop flow (SUI -> USDC -> WAL) via MoveVM router quote
//!
//! Run with:
//! `cargo run --example full_deepbook_flow`

use anyhow::{anyhow, Context, Result};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::RwLock;

use deepbook_sandbox_backend::sandbox::orderbook_builder::{OrderbookBuilder, SandboxOrderbook};
use deepbook_sandbox_backend::sandbox::router::{self, RouterHandle};
use deepbook_sandbox_backend::sandbox::state_loader::{DeepBookConfig, PoolId, StateLoader};
use deepbook_sandbox_backend::sandbox::swap_executor::{
    CommandInfo, EventInfo, PtbExecution, SessionManager, TradingSession,
};

const DIRECT_SWAP_SUI_AMOUNT: u64 = 10_000_000_000; // 10 SUI
const TWO_HOP_SWAP_SUI_AMOUNT: u64 = 5_000_000_000; // 5 SUI

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt().with_env_filter("info").init();
    dotenvy::dotenv().ok();

    let data_dir = resolve_data_dir()?;
    println!("Using data directory: {}", data_dir.display());

    let (orderbooks, pool_files) = build_all_orderbooks(&data_dir).await?;

    println!();
    println!("Built {} MoveVM orderbooks.", orderbooks.len());
    for (pool_id, ob) in &orderbooks {
        println!(
            "  {} => bids: {}, asks: {}, mid: ${:.6}",
            pool_id.display_name(),
            ob.bids.len(),
            ob.asks.len(),
            ob.mid_price().unwrap_or(0.0)
        );
    }

    let session_manager = Arc::new(SessionManager::new(orderbooks));
    let session_id = session_manager.create_session().await?;
    let session = session_manager
        .get_session(&session_id)
        .await
        .ok_or_else(|| anyhow!("Failed to fetch newly created session"))?;

    println!();
    println!("Session created: {}", session_id);

    let (router_handle, ready_rx) = router::spawn_router_thread(pool_files.clone());
    match ready_rx.await {
        Ok(Ok(())) => {
            println!("Router thread ready for MoveVM quoting.");
        }
        Ok(Err(e)) => {
            return Err(anyhow!("Router setup failed: {}", e));
        }
        Err(_) => {
            return Err(anyhow!("Router setup channel dropped"));
        }
    }

    run_direct_swap(&session, &router_handle).await?;
    run_two_hop_swap(&session, &router_handle).await?;

    let final_session = session.read().await;
    println!();
    println!("Final balances:");
    println!(
        "  SUI:  {:.6}",
        final_session.balances.sui as f64 / 1_000_000_000.0
    );
    println!(
        "  USDC: {:.6}",
        final_session.balances.usdc as f64 / 1_000_000.0
    );
    println!(
        "  DEEP: {:.6}",
        final_session.balances.deep as f64 / 1_000_000.0
    );
    println!(
        "  WAL:  {:.6}",
        final_session.balances.wal as f64 / 1_000_000_000.0
    );
    println!("Swap history entries: {}", final_session.swap_history.len());

    Ok(())
}

fn resolve_data_dir() -> Result<PathBuf> {
    let candidates = ["data", "backend/data"];
    for candidate in candidates {
        let path = Path::new(candidate);
        if path.is_dir() {
            return Ok(path.to_path_buf());
        }
    }
    Err(anyhow!(
        "Could not find data directory. Expected `data/` or `backend/data/`."
    ))
}

fn pool_specs() -> [(PoolId, &'static str); 3] {
    [
        (PoolId::SuiUsdc, "sui_usdc_state_cp240M.jsonl"),
        (PoolId::WalUsdc, "wal_usdc_state_cp240M.jsonl"),
        (PoolId::DeepUsdc, "deep_usdc_state_cp240M.jsonl"),
    ]
}

struct BuiltPool {
    orderbook: SandboxOrderbook,
    loaded_rows: usize,
    unique_objects: usize,
    checkpoint: u64,
}

fn build_pool_orderbook(pool_id: PoolId, path: PathBuf) -> Result<BuiltPool> {
    let config = DeepBookConfig::for_pool(pool_id);
    let mut loader = StateLoader::with_config(config.clone());
    let loaded_rows = loader
        .load_from_file(&path)
        .map_err(|e| anyhow!("failed to load {}: {}", path.display(), e))?;
    let stats = loader.stats();

    // Use a dedicated runtime here to avoid dropping nested runtimes from async context.
    let rt = tokio::runtime::Runtime::new()?;
    let mut builder = OrderbookBuilder::new()?;
    rt.block_on(builder.load_packages_from_grpc())?;
    builder.load_pool_state(&loader, pool_id)?;
    let orderbook = builder
        .build_orderbook(pool_id, &config.pool_wrapper, stats.max_checkpoint)
        .with_context(|| format!("failed to build orderbook for {}", pool_id.display_name()))?;

    // Drop in this order to avoid runtime shutdown panic.
    drop(builder);
    drop(rt);

    Ok(BuiltPool {
        orderbook,
        loaded_rows,
        unique_objects: stats.total_objects,
        checkpoint: stats.max_checkpoint,
    })
}

async fn build_all_orderbooks(
    data_dir: &Path,
) -> Result<(HashMap<PoolId, SandboxOrderbook>, Vec<(PoolId, String)>)> {
    let mut orderbooks = HashMap::new();
    let mut pool_files = Vec::new();

    for (pool_id, file_name) in pool_specs() {
        let path = data_dir.join(file_name);
        if !path.exists() {
            return Err(anyhow!(
                "Missing pool state file for {}: {}",
                pool_id.display_name(),
                path.display()
            ));
        }

        let absolute_path = std::fs::canonicalize(&path).unwrap_or(path.clone());
        pool_files.push((pool_id, absolute_path.to_string_lossy().to_string()));

        println!();
        println!(
            "[{}] Loading state from {}",
            pool_id.display_name(),
            path.display()
        );
        let path_for_build = path.clone();
        let built =
            tokio::task::spawn_blocking(move || build_pool_orderbook(pool_id, path_for_build))
                .await
                .map_err(|e| {
                    anyhow!(
                        "orderbook build task failed for {}: {}",
                        pool_id.display_name(),
                        e
                    )
                })??;
        println!(
            "  Loaded {} JSONL rows ({} unique objects), checkpoint {}",
            built.loaded_rows, built.unique_objects, built.checkpoint
        );

        println!(
            "  Built orderbook: {} bids / {} asks",
            built.orderbook.bids.len(),
            built.orderbook.asks.len()
        );

        orderbooks.insert(pool_id, built.orderbook);
    }

    Ok((orderbooks, pool_files))
}

async fn run_direct_swap(
    session: &Arc<RwLock<TradingSession>>,
    router_handle: &RouterHandle,
) -> Result<()> {
    let from = "SUI";
    let to = "USDC";
    let pool_id = PoolId::SuiUsdc;
    let input_amount = DIRECT_SWAP_SUI_AMOUNT;
    let start = std::time::Instant::now();
    let deep_budget = { session.read().await.balances.deep };

    let swap_vm = router_handle
        .execute_single_hop_swap(pool_id, input_amount, deep_budget, true)
        .await
        .map_err(|e| anyhow!("MoveVM single-hop swap failed: {}", e))?;
    if swap_vm.output_amount == 0 {
        return Err(anyhow!("Direct MoveVM swap returned zero output"));
    }
    let consumed_input = input_amount.saturating_sub(swap_vm.input_refund);
    let input_human = format_human(consumed_input, get_decimals(from));
    let output_human = format_human(swap_vm.output_amount, get_decimals(to));
    let effective_price = if input_human > 0.0 {
        output_human / input_human
    } else {
        0.0
    };

    println!();
    println!(
        "Direct swap VM output: {:.6} {} (requested {:.6}) -> {:.6} {}",
        format_human(consumed_input, get_decimals(from)),
        from,
        format_human(input_amount, get_decimals(from)),
        format_human(swap_vm.output_amount, get_decimals(to)),
        to
    );

    let ptb_execution = PtbExecution {
        commands: vec![
            CommandInfo {
                index: 0,
                command_type: "MoveCall".to_string(),
                package: "0x2c8d603bc51326b8c13cef9dd07031a408a48dddb541963357661df5d3204809"
                    .to_string(),
                module: "pool".to_string(),
                function: "swap_exact_base_for_quote".to_string(),
                type_args: vec![],
            },
            CommandInfo {
                index: 1,
                command_type: "MoveCall".to_string(),
                package: "0x2".to_string(),
                module: "coin".to_string(),
                function: "value".to_string(),
                type_args: vec![],
            },
            CommandInfo {
                index: 2,
                command_type: "MoveCall".to_string(),
                package: "0x2".to_string(),
                module: "coin".to_string(),
                function: "value".to_string(),
                type_args: vec![],
            },
            CommandInfo {
                index: 3,
                command_type: "MoveCall".to_string(),
                package: "0x2".to_string(),
                module: "coin".to_string(),
                function: "value".to_string(),
                type_args: vec![],
            },
            CommandInfo {
                index: 4,
                command_type: "TransferObjects".to_string(),
                package: "0x2".to_string(),
                module: "transfer".to_string(),
                function: "public_transfer".to_string(),
                type_args: vec![],
            },
        ],
        status: "Success".to_string(),
        effects_digest: None,
        events: swap_vm
            .events
            .iter()
            .map(|e| EventInfo {
                event_type: e.event_type.clone(),
                data: serde_json::json!({ "bcs": e.data_hex }),
            })
            .collect(),
        created_objects: vec![],
        mutated_objects: vec![pool_id.display_name().to_string()],
        deleted_objects: vec![],
    };

    let mut session = session.write().await;
    let swap = session.apply_vm_swap(
        from,
        to,
        input_amount,
        swap_vm.input_refund,
        deep_budget,
        swap_vm.deep_refund,
        swap_vm.output_amount,
        effective_price,
        swap_vm.gas_used,
        start.elapsed().as_millis() as u64,
        ptb_execution,
    )?;

    if !swap.success {
        return Err(anyhow!(
            "Direct swap failed: {}",
            swap.error.unwrap_or_else(|| "unknown error".to_string())
        ));
    }

    println!(
        "Direct swap executed: {:.6} {} -> {:.6} {}",
        format_human(consumed_input, get_decimals(from)),
        from,
        format_human(swap.output_amount, get_decimals(to)),
        to
    );

    Ok(())
}

async fn run_two_hop_swap(
    session: &Arc<RwLock<TradingSession>>,
    router_handle: &RouterHandle,
) -> Result<()> {
    let from = "SUI";
    let to = "WAL";
    let amount = TWO_HOP_SWAP_SUI_AMOUNT;
    let first_pool = PoolId::SuiUsdc;
    let second_pool = PoolId::WalUsdc;
    let start = std::time::Instant::now();
    let deep_budget = { session.read().await.balances.deep };

    let swap_vm = router_handle
        .execute_two_hop_swap(first_pool, second_pool, amount, deep_budget)
        .await
        .map_err(|e| anyhow!("MoveVM two-hop swap failed: {}", e))?;
    let intermediate_usdc = swap_vm.intermediate_amount;
    let output_amount = swap_vm.output_amount;

    if output_amount == 0 {
        return Err(anyhow!("Two-hop swap returned zero output"));
    }
    let consumed_input = amount.saturating_sub(swap_vm.input_refund);

    println!(
        "Two-hop swap VM output: {:.6} {} (requested {:.6}) -> {:.6} USDC -> {:.6} {}",
        format_human(consumed_input, get_decimals(from)),
        from,
        format_human(amount, get_decimals(from)),
        format_human(intermediate_usdc, get_decimals("USDC")),
        format_human(output_amount, get_decimals(to)),
        to
    );

    let ptb_execution = PtbExecution {
        commands: vec![
            CommandInfo {
                index: 0,
                command_type: "MoveCall".to_string(),
                package: "0x2c8d603bc51326b8c13cef9dd07031a408a48dddb541963357661df5d3204809"
                    .to_string(),
                module: "pool".to_string(),
                function: "swap_exact_base_for_quote".to_string(),
                type_args: vec![],
            },
            CommandInfo {
                index: 1,
                command_type: "MoveCall".to_string(),
                package: "0x2".to_string(),
                module: "coin".to_string(),
                function: "value".to_string(),
                type_args: vec![],
            },
            CommandInfo {
                index: 2,
                command_type: "MoveCall".to_string(),
                package: "0x2c8d603bc51326b8c13cef9dd07031a408a48dddb541963357661df5d3204809"
                    .to_string(),
                module: "pool".to_string(),
                function: "swap_exact_quote_for_base".to_string(),
                type_args: vec![],
            },
            CommandInfo {
                index: 3,
                command_type: "MoveCall".to_string(),
                package: "0x2".to_string(),
                module: "coin".to_string(),
                function: "value".to_string(),
                type_args: vec![],
            },
            CommandInfo {
                index: 4,
                command_type: "MoveCall".to_string(),
                package: "0x2".to_string(),
                module: "coin".to_string(),
                function: "value".to_string(),
                type_args: vec![],
            },
            CommandInfo {
                index: 5,
                command_type: "MoveCall".to_string(),
                package: "0x2".to_string(),
                module: "coin".to_string(),
                function: "value".to_string(),
                type_args: vec![],
            },
            CommandInfo {
                index: 6,
                command_type: "MoveCall".to_string(),
                package: "0x2".to_string(),
                module: "coin".to_string(),
                function: "value".to_string(),
                type_args: vec![],
            },
            CommandInfo {
                index: 7,
                command_type: "TransferObjects".to_string(),
                package: "0x2".to_string(),
                module: "transfer".to_string(),
                function: "public_transfer".to_string(),
                type_args: vec![],
            },
        ],
        status: "Success".to_string(),
        effects_digest: None,
        events: swap_vm
            .events
            .iter()
            .map(|e| EventInfo {
                event_type: e.event_type.clone(),
                data: serde_json::json!({ "bcs": e.data_hex }),
            })
            .collect(),
        created_objects: vec![],
        mutated_objects: vec![
            first_pool.display_name().to_string(),
            second_pool.display_name().to_string(),
        ],
        deleted_objects: vec![],
    };

    let mut session = session.write().await;
    let input_human = format_human(consumed_input, get_decimals(from));
    let output_human = format_human(output_amount, get_decimals(to));
    let effective_price = if input_human > 0.0 {
        output_human / input_human
    } else {
        0.0
    };

    let swap = session.apply_vm_swap(
        from,
        to,
        amount,
        swap_vm.input_refund,
        deep_budget,
        swap_vm.deep_refund,
        output_amount,
        effective_price,
        swap_vm.gas_used,
        start.elapsed().as_millis() as u64,
        ptb_execution,
    )?;

    if !swap.success {
        return Err(anyhow!(
            "Two-hop swap failed: {}",
            swap.error.unwrap_or_else(|| "unknown error".to_string())
        ));
    }

    println!(
        "Two-hop swap executed: {:.6} {} -> {:.6} {}",
        format_human(consumed_input, get_decimals(from)),
        from,
        format_human(swap.output_amount, get_decimals(to)),
        to
    );

    Ok(())
}

fn get_decimals(token: &str) -> i32 {
    match token {
        "SUI" | "WAL" => 9,
        "USDC" | "DEEP" => 6,
        _ => 9,
    }
}

fn format_human(amount: u64, decimals: i32) -> f64 {
    amount as f64 / 10f64.powi(decimals)
}
