//! Sui sandbox integration
//!
//! Wraps sui-sandbox SimulationEnvironment for HTTP API usage.
//! Handles:
//! - Loading forked DeepBook state from Snowflake JSON exports
//! - Converting JSON to BCS using bytecode layouts
//! - Managing SimulationEnvironment instances per session
//! - Calling DeepBook view functions via Move VM

pub mod orderbook_builder;
pub mod snowflake_bcs;
pub mod state_loader;
pub mod swap_executor;
