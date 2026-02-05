//! Test all three DeepBook pools at checkpoint 240M
//!
//! Run with: cargo run --example test_all_pools_240m

use anyhow::Result;
use std::path::Path;

use deepbook_sandbox_backend::sandbox::orderbook_builder::OrderbookBuilder;
use deepbook_sandbox_backend::sandbox::state_loader::{DeepBookConfig, PoolId, StateLoader};

fn test_pool(pool_id: PoolId, data_file: &str, pool_wrapper: &str) -> Result<()> {
    let config = DeepBookConfig::for_pool(pool_id);

    println!("\n{}", "=".repeat(60));
    println!("Testing {} pool", pool_id.display_name());
    println!("{}", "=".repeat(60));
    println!("Data file: {}", data_file);
    println!("Pool wrapper: {}", pool_wrapper);

    // Check if file exists
    let data_path = Path::new(data_file);
    if !data_path.exists() {
        println!("  ❌ Data file not found: {}", data_file);
        return Ok(());
    }

    // Create runtime for async operations
    let rt = tokio::runtime::Runtime::new()?;

    // Create builder and load packages
    let mut builder = OrderbookBuilder::new()?;
    rt.block_on(builder.load_packages_from_grpc())?;

    // Load pool state
    let mut loader = StateLoader::with_config(config);
    loader
        .load_from_file(data_path)
        .map_err(|e| anyhow::anyhow!("Failed to load state: {}", e))?;

    let stats = loader.stats();
    println!("  Objects loaded: {}", stats.total_objects);
    println!("  Checkpoint: {}", stats.max_checkpoint);

    // Load state into builder
    builder.load_pool_state(&loader, pool_id)?;

    // Build orderbook
    match builder.build_orderbook(pool_id, pool_wrapper, stats.max_checkpoint) {
        Ok(orderbook) => {
            println!("\n  ✓ Orderbook built successfully!");

            // Calculate price divisor: USDC (10^6) * normalization (10^(9 - base_decimals))
            let price_divisor = 1_000_000.0 * 10f64.powi(9 - orderbook.base_decimals as i32);
            let qty_divisor = 10f64.powi(orderbook.base_decimals as i32);

            println!("\n  Bids (top 5):");
            for (i, level) in orderbook.bids.iter().take(5).enumerate() {
                let price = level.price as f64 / price_divisor;
                let qty = level.total_quantity as f64 / qty_divisor;
                println!(
                    "    {}. ${:.6} - {:.4} ({} orders)",
                    i + 1,
                    price,
                    qty,
                    level.order_count
                );
            }

            println!("\n  Asks (top 5):");
            for (i, level) in orderbook.asks.iter().take(5).enumerate() {
                let price = level.price as f64 / price_divisor;
                let qty = level.total_quantity as f64 / qty_divisor;
                println!(
                    "    {}. ${:.6} - {:.4} ({} orders)",
                    i + 1,
                    price,
                    qty,
                    level.order_count
                );
            }

            println!("\n  Summary:");
            println!("    Total bid levels: {}", orderbook.bids.len());
            println!("    Total ask levels: {}", orderbook.asks.len());

            if let Some(mid) = orderbook.mid_price() {
                println!("    Mid price: ${:.6}", mid);
            }
            if let Some(spread) = orderbook.spread_bps() {
                println!("    Spread: {} bps", spread);
            }

            // Check for crossed book
            if let (Some(best_bid), Some(best_ask)) = (orderbook.best_bid(), orderbook.best_ask()) {
                if best_bid >= best_ask {
                    println!("    ⚠️  WARNING: Crossed book!");
                } else {
                    println!("    ✓ Book valid (bid < ask)");
                }
            }
        }
        Err(e) => {
            println!("\n  ❌ Failed to build orderbook: {}", e);
        }
    }

    drop(builder);
    drop(rt);

    Ok(())
}

fn main() -> Result<()> {
    tracing_subscriber::fmt().with_env_filter("warn").init();

    println!("=================================================================");
    println!("DeepBook Move VM Test - All Pools at Checkpoint 240M");
    println!("=================================================================");

    // Test all three pools
    let pools = [
        (
            PoolId::SuiUsdc,
            "data/sui_usdc_state_cp240M.jsonl",
            "0xe05dafb5133bcffb8d59f4e12465dc0e9faeaa05e3e342a08fe135800e3e4407",
        ),
        (
            PoolId::DeepUsdc,
            "data/deep_usdc_state_cp240M.jsonl",
            "0xf948981b806057580f91622417534f491da5f61aeaf33d0ed8e69fd5691c95ce",
        ),
        (
            PoolId::WalUsdc,
            "data/wal_usdc_state_cp240M.jsonl",
            "0x56a1c985c1f1123181d6b881714793689321ba24301b3585eec427436eb1c76d",
        ),
    ];

    for (pool_id, data_file, pool_wrapper) in pools {
        if let Err(e) = test_pool(pool_id, data_file, pool_wrapper) {
            println!("  Error testing {}: {}", pool_id.display_name(), e);
        }
    }

    println!("\n=================================================================");
    println!("Test Complete");
    println!("=================================================================");

    Ok(())
}
