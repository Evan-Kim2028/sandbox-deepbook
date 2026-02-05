//! Test the orderbook builder flow end-to-end
//!
//! Run with: cargo run --example test_orderbook

use anyhow::Result;
use std::path::Path;

// Re-use the backend's modules
use deepbook_sandbox_backend::sandbox::orderbook_builder::OrderbookBuilder;
use deepbook_sandbox_backend::sandbox::state_loader::{DeepBookConfig, PoolId, StateLoader};

fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt().with_env_filter("info").init();

    println!("=== DeepBook Orderbook Builder Test ===\n");

    // Use a dedicated tokio runtime for the async operations
    // This prevents the "cannot drop runtime in async context" panic
    let rt = tokio::runtime::Runtime::new()?;

    // Step 1: Create the orderbook builder
    println!("1. Creating OrderbookBuilder...");
    let mut builder = OrderbookBuilder::new()?;
    println!("   ✓ Builder created\n");

    // Step 2: Load packages from gRPC (this requires network)
    println!("2. Loading packages from gRPC (Move Stdlib, Sui Framework, DeepBook)...");
    rt.block_on(builder.load_packages_from_grpc())?;
    println!("   ✓ Packages loaded\n");

    // Step 3: Load pool state from JSONL file
    // Using checkpoint 240M with complete state (10 slices per side)
    let data_path = Path::new("data/sui_usdc_state_cp240M.jsonl");
    println!("3. Loading pool state from {}...", data_path.display());

    let config = DeepBookConfig::for_pool(PoolId::SuiUsdc);
    let mut loader = StateLoader::with_config(config);
    loader
        .load_from_file(data_path)
        .map_err(|e| anyhow::anyhow!("Failed to load state: {}", e))?;

    let stats = loader.stats();
    println!("   Objects loaded: {}", stats.total_objects);
    println!("   Checkpoint: {}\n", stats.max_checkpoint);

    // Step 4: Load the pool state into the builder
    println!("4. Converting JSON to BCS and loading into sandbox...");
    builder.load_pool_state(&loader, PoolId::SuiUsdc)?;
    println!("   ✓ Pool state loaded\n");

    // Step 5: Build the orderbook by calling iter_orders
    let pool_wrapper = "0xe05dafb5133bcffb8d59f4e12465dc0e9faeaa05e3e342a08fe135800e3e4407";
    let checkpoint = stats.max_checkpoint;

    println!("5. Calling deepbook::order_query::iter_orders via PTB...");
    println!("   Pool: {}", pool_wrapper);

    match builder.build_orderbook(PoolId::SuiUsdc, pool_wrapper, checkpoint) {
        Ok(orderbook) => {
            println!("\n=== ORDERBOOK RESULTS ===\n");

            println!("Bids (top 10):");
            for (i, level) in orderbook.bids.iter().take(10).enumerate() {
                let price = level.price as f64 / 1_000_000.0;
                let qty = level.total_quantity as f64 / 1_000_000_000.0;
                println!(
                    "  {}. ${:.6} - {:.4} SUI ({} orders)",
                    i + 1,
                    price,
                    qty,
                    level.order_count
                );
            }

            println!("\nAsks (top 10):");
            for (i, level) in orderbook.asks.iter().take(10).enumerate() {
                let price = level.price as f64 / 1_000_000.0;
                let qty = level.total_quantity as f64 / 1_000_000_000.0;
                println!(
                    "  {}. ${:.6} - {:.4} SUI ({} orders)",
                    i + 1,
                    price,
                    qty,
                    level.order_count
                );
            }

            println!("\nSummary:");
            println!("  Total bid levels: {}", orderbook.bids.len());
            println!("  Total ask levels: {}", orderbook.asks.len());

            if let Some(mid) = orderbook.mid_price() {
                println!("  Mid price: ${:.6}", mid);
            }
            if let Some(spread) = orderbook.spread_bps() {
                println!("  Spread: {} bps", spread);
            }

            // Verify no crossed book
            if let (Some(best_bid), Some(best_ask)) = (orderbook.best_bid(), orderbook.best_ask()) {
                if best_bid >= best_ask {
                    println!(
                        "\n  ⚠️  WARNING: Crossed book detected! Bid ${} >= Ask ${}",
                        best_bid, best_ask
                    );
                } else {
                    println!("\n  ✓ Book is valid (best bid < best ask)");
                }
            }
        }
        Err(e) => {
            println!("\n❌ Failed to build orderbook: {}", e);
            println!("\nThis might be due to:");
            println!("  - Missing dynamic field slices (BigVector children)");
            println!("  - BCS conversion errors");
            println!("  - iter_orders function signature mismatch");
        }
    }

    // Explicitly drop in the right order - builder first (contains nested runtime),
    // then our outer runtime
    drop(builder);
    drop(rt);

    Ok(())
}
