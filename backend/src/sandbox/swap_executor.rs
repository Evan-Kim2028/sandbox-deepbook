//! DeepBook Swap Executor for Session-Based Trading
//!
//! Manages trading sessions with user balances and swap execution.
//! Uses the loaded pool state for quote calculation.

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use super::orderbook_builder::SandboxOrderbook;
use super::state_loader::PoolId;

// Initial balances for new sessions
const INITIAL_SUI: u64 = 100_000_000_000; // 100 SUI
const INITIAL_USDC: u64 = 1_000_000_000; // 1000 USDC
const INITIAL_DEEP: u64 = 100_000_000; // 100 DEEP
const INITIAL_WAL: u64 = 10_000_000_000; // 10 WAL

// DeepBook V3 Package
const DEEPBOOK_PACKAGE: &str = "0x2c8d603bc51326b8c13cef9dd07031a408a48dddb541963357661df5d3204809";

// Sui Framework
const SUI_FRAMEWORK: &str = "0x2";

// Type tags
const SUI_TYPE: &str = "0x2::sui::SUI";
const USDC_TYPE: &str =
    "0xdba34672e30cb065b1f93e3ab55318768fd6fef66c15942c9f7cb846e2f900e7::usdc::USDC";
const WAL_TYPE: &str =
    "0x356a26eb9e012a68958082340d4c4116e7f55615cf27affcff209cf0ae544f59::wal::WAL";
const DEEP_TYPE: &str =
    "0xdeeb7a4662eec9f2f3def03fb937a663dddaa2e215b8078a284d026b7946c270::deep::DEEP";

/// Result of a swap execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwapResult {
    pub success: bool,
    pub error: Option<String>,
    pub input_token: String,
    pub output_token: String,
    pub input_amount: u64,
    pub output_amount: u64,
    pub effective_price: f64,
    pub gas_used: u64,
    pub execution_time_ms: u64,
    pub ptb_execution: PtbExecution,
    pub balances_after: UserBalances,
}

/// PTB execution details
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PtbExecution {
    pub commands: Vec<CommandInfo>,
    pub status: String,
    pub effects_digest: Option<String>,
    pub events: Vec<EventInfo>,
    pub created_objects: Vec<String>,
    pub mutated_objects: Vec<String>,
    pub deleted_objects: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandInfo {
    pub index: usize,
    pub command_type: String,
    pub package: String,
    pub module: String,
    pub function: String,
    pub type_args: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventInfo {
    pub event_type: String,
    pub data: serde_json::Value,
}

/// User's token balances
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UserBalances {
    pub sui: u64,
    pub usdc: u64,
    pub deep: u64,
    pub wal: u64,
}

impl UserBalances {
    pub fn initial() -> Self {
        Self {
            sui: INITIAL_SUI,
            usdc: INITIAL_USDC,
            deep: INITIAL_DEEP,
            wal: INITIAL_WAL,
        }
    }

    pub fn get(&self, token: &str) -> u64 {
        match token.to_uppercase().as_str() {
            "SUI" => self.sui,
            "USDC" => self.usdc,
            "DEEP" => self.deep,
            "WAL" => self.wal,
            _ => 0,
        }
    }

    pub fn set(&mut self, token: &str, amount: u64) {
        match token.to_uppercase().as_str() {
            "SUI" => self.sui = amount,
            "USDC" => self.usdc = amount,
            "DEEP" => self.deep = amount,
            "WAL" => self.wal = amount,
            _ => {}
        }
    }

    pub fn subtract(&mut self, token: &str, amount: u64) -> Result<()> {
        let current = self.get(token);
        if current < amount {
            return Err(anyhow!(
                "Insufficient {} balance: have {}, need {}",
                token,
                current,
                amount
            ));
        }
        self.set(token, current - amount);
        Ok(())
    }

    pub fn add(&mut self, token: &str, amount: u64) {
        let current = self.get(token);
        self.set(token, current + amount);
    }
}

/// A trading session with user state
pub struct TradingSession {
    pub created_at: std::time::Instant,
    pub balances: UserBalances,
    pub swap_history: Vec<SwapResult>,
    pub checkpoint: u64,
    /// Per-session orderbook clones (modified by swaps)
    pub orderbooks: HashMap<PoolId, SandboxOrderbook>,
}

impl TradingSession {
    /// Create a new trading session with cloned orderbooks
    pub fn new(_session_id: String, orderbooks: HashMap<PoolId, SandboxOrderbook>) -> Result<Self> {
        Ok(Self {
            created_at: std::time::Instant::now(),
            balances: UserBalances::initial(),
            swap_history: Vec::new(),
            checkpoint: 240_000_000, // Default to checkpoint 240M
            orderbooks,
        })
    }

    /// Execute a swap using the DeepBook orderbook
    ///
    /// `output_amount` must be pre-calculated by walking the MoveVM-built orderbook.
    /// `effective_price` is the price from the orderbook walk (quote per base).
    pub fn execute_swap(
        &mut self,
        pool_id: PoolId,
        from_token: &str,
        to_token: &str,
        input_amount: u64,
        output_amount: u64,
        effective_price: f64,
    ) -> Result<SwapResult> {
        let start = std::time::Instant::now();

        // Validate balance
        if self.balances.get(from_token) < input_amount {
            return Ok(SwapResult {
                success: false,
                error: Some(format!(
                    "Insufficient {} balance: have {}, need {}",
                    from_token,
                    self.balances.get(from_token),
                    input_amount
                )),
                input_token: from_token.to_string(),
                output_token: to_token.to_string(),
                input_amount,
                output_amount: 0,
                effective_price: 0.0,
                gas_used: 0,
                execution_time_ms: start.elapsed().as_millis() as u64,
                ptb_execution: PtbExecution {
                    commands: vec![],
                    status: "Failed: Insufficient balance".to_string(),
                    effects_digest: None,
                    events: vec![],
                    created_objects: vec![],
                    mutated_objects: vec![],
                    deleted_objects: vec![],
                },
                balances_after: self.balances.clone(),
            });
        }

        // Determine swap direction
        let is_sell_base = from_token.to_uppercase() != "USDC";

        // Get token info
        let (base_type, quote_type, _base_decimals) = match pool_id {
            PoolId::SuiUsdc => (SUI_TYPE, USDC_TYPE, 9u8),
            PoolId::DeepUsdc => (DEEP_TYPE, USDC_TYPE, 6u8),
            PoolId::WalUsdc => (WAL_TYPE, USDC_TYPE, 9u8),
        };

        // Update balances
        self.balances.subtract(from_token, input_amount)?;
        self.balances.add(to_token, output_amount);

        let execution_time = start.elapsed().as_millis() as u64;

        // Build PTB execution info
        let ptb_execution = PtbExecution {
            commands: vec![
                CommandInfo {
                    index: 0,
                    command_type: "SplitCoins".to_string(),
                    package: SUI_FRAMEWORK.to_string(),
                    module: "coin".to_string(),
                    function: "split".to_string(),
                    type_args: vec![if is_sell_base {
                        base_type.to_string()
                    } else {
                        quote_type.to_string()
                    }],
                },
                CommandInfo {
                    index: 1,
                    command_type: "MoveCall".to_string(),
                    package: DEEPBOOK_PACKAGE.to_string(),
                    module: "pool".to_string(),
                    function: if is_sell_base {
                        "swap_exact_base_for_quote".to_string()
                    } else {
                        "swap_exact_quote_for_base".to_string()
                    },
                    type_args: vec![base_type.to_string(), quote_type.to_string()],
                },
                CommandInfo {
                    index: 2,
                    command_type: "TransferObjects".to_string(),
                    package: SUI_FRAMEWORK.to_string(),
                    module: "transfer".to_string(),
                    function: "public_transfer".to_string(),
                    type_args: vec![if is_sell_base {
                        quote_type.to_string()
                    } else {
                        base_type.to_string()
                    }],
                },
            ],
            status: "Success".to_string(),
            effects_digest: Some(format!("SimDigest_{}", uuid::Uuid::new_v4())),
            events: vec![EventInfo {
                event_type: format!("{}::pool::OrderFilled", DEEPBOOK_PACKAGE),
                data: serde_json::json!({
                    "pool_id": pool_id.display_name(),
                    "direction": if is_sell_base { format!("Sell {} for {}", from_token, to_token) } else { format!("Buy {} with {}", to_token, from_token) },
                    "taker_is_bid": !is_sell_base,
                    "base_token": if is_sell_base { from_token } else { to_token },
                    "quote_token": "USDC",
                    "base_quantity": if is_sell_base { input_amount } else { output_amount },
                    "quote_quantity": if is_sell_base { output_amount } else { input_amount },
                    "base_quantity_human": if is_sell_base {
                        format!("{:.4}", input_amount as f64 / 10f64.powi(_base_decimals as i32))
                    } else {
                        format!("{:.4}", output_amount as f64 / 10f64.powi(_base_decimals as i32))
                    },
                    "quote_quantity_human": format!("{:.2}", if is_sell_base { output_amount } else { input_amount } as f64 / 1_000_000.0),
                }),
            }],
            created_objects: vec![],
            mutated_objects: vec![
                format!("UserCoin<{}>", from_token),
                format!("UserCoin<{}>", to_token),
            ],
            deleted_objects: vec![],
        };

        let result = SwapResult {
            success: true,
            error: None,
            input_token: from_token.to_string(),
            output_token: to_token.to_string(),
            input_amount,
            output_amount,
            effective_price,
            gas_used: 1_500_000, // Simulated gas
            execution_time_ms: execution_time,
            ptb_execution,
            balances_after: self.balances.clone(),
        };

        // Add to history
        self.swap_history.push(result.clone());

        Ok(result)
    }

    /// Execute a two-hop swap: from_token -> USDC -> to_token
    ///
    /// Both legs are pre-calculated. This method updates balances and builds
    /// the PTB execution info showing a router::swap_two_hop call.
    pub fn execute_two_hop_swap(
        &mut self,
        first_pool: PoolId,
        second_pool: PoolId,
        from_token: &str,
        to_token: &str,
        input_amount: u64,
        intermediate_usdc: u64,
        output_amount: u64,
        effective_price: f64,
    ) -> Result<SwapResult> {
        let start = std::time::Instant::now();

        // Validate balance
        if self.balances.get(from_token) < input_amount {
            return Ok(SwapResult {
                success: false,
                error: Some(format!(
                    "Insufficient {} balance: have {}, need {}",
                    from_token,
                    self.balances.get(from_token),
                    input_amount
                )),
                input_token: from_token.to_string(),
                output_token: to_token.to_string(),
                input_amount,
                output_amount: 0,
                effective_price: 0.0,
                gas_used: 0,
                execution_time_ms: start.elapsed().as_millis() as u64,
                ptb_execution: PtbExecution {
                    commands: vec![],
                    status: "Failed: Insufficient balance".to_string(),
                    effects_digest: None,
                    events: vec![],
                    created_objects: vec![],
                    mutated_objects: vec![],
                    deleted_objects: vec![],
                },
                balances_after: self.balances.clone(),
            });
        }

        // Get type info for both pools
        let (first_base_type, _, first_base_decimals) = match first_pool {
            PoolId::SuiUsdc => (SUI_TYPE, USDC_TYPE, 9u8),
            PoolId::DeepUsdc => (DEEP_TYPE, USDC_TYPE, 6u8),
            PoolId::WalUsdc => (WAL_TYPE, USDC_TYPE, 9u8),
        };
        let (second_base_type, _, second_base_decimals) = match second_pool {
            PoolId::SuiUsdc => (SUI_TYPE, USDC_TYPE, 9u8),
            PoolId::DeepUsdc => (DEEP_TYPE, USDC_TYPE, 6u8),
            PoolId::WalUsdc => (WAL_TYPE, USDC_TYPE, 9u8),
        };

        // Update balances atomically
        self.balances.subtract(from_token, input_amount)?;
        self.balances.add(to_token, output_amount);

        let execution_time = start.elapsed().as_millis() as u64;

        // Build PTB execution info showing atomic router call
        let ptb_execution = PtbExecution {
            commands: vec![
                CommandInfo {
                    index: 0,
                    command_type: "SplitCoins".to_string(),
                    package: SUI_FRAMEWORK.to_string(),
                    module: "coin".to_string(),
                    function: "split".to_string(),
                    type_args: vec![first_base_type.to_string()],
                },
                CommandInfo {
                    index: 1,
                    command_type: "MoveCall".to_string(),
                    package: "router".to_string(),
                    module: "router".to_string(),
                    function: "swap_two_hop".to_string(),
                    type_args: vec![
                        first_base_type.to_string(),
                        USDC_TYPE.to_string(),
                        second_base_type.to_string(),
                    ],
                },
                CommandInfo {
                    index: 2,
                    command_type: "TransferObjects".to_string(),
                    package: SUI_FRAMEWORK.to_string(),
                    module: "transfer".to_string(),
                    function: "public_transfer".to_string(),
                    type_args: vec![second_base_type.to_string()],
                },
            ],
            status: "Success".to_string(),
            effects_digest: Some(format!("SimDigest_{}", uuid::Uuid::new_v4())),
            events: vec![
                EventInfo {
                    event_type: format!("{}::pool::OrderFilled", DEEPBOOK_PACKAGE),
                    data: serde_json::json!({
                        "pool_id": first_pool.display_name(),
                        "leg": "first",
                        "direction": format!("Sell {} for USDC", from_token),
                        "taker_is_bid": false,
                        "base_token": from_token,
                        "quote_token": "USDC",
                        "base_quantity": input_amount,
                        "quote_quantity": intermediate_usdc,
                        "base_quantity_human": format!("{:.4}", input_amount as f64 / 10f64.powi(first_base_decimals as i32)),
                        "quote_quantity_human": format!("{:.2}", intermediate_usdc as f64 / 1_000_000.0),
                    }),
                },
                EventInfo {
                    event_type: format!("{}::pool::OrderFilled", DEEPBOOK_PACKAGE),
                    data: serde_json::json!({
                        "pool_id": second_pool.display_name(),
                        "leg": "second",
                        "direction": format!("Buy {} with USDC", to_token),
                        "taker_is_bid": true,
                        "base_token": to_token,
                        "quote_token": "USDC",
                        "base_quantity": output_amount,
                        "quote_quantity": intermediate_usdc,
                        "base_quantity_human": format!("{:.4}", output_amount as f64 / 10f64.powi(second_base_decimals as i32)),
                        "quote_quantity_human": format!("{:.2}", intermediate_usdc as f64 / 1_000_000.0),
                    }),
                },
            ],
            created_objects: vec![],
            mutated_objects: vec![
                format!("UserCoin<{}>", from_token),
                format!("UserCoin<{}>", to_token),
            ],
            deleted_objects: vec![],
        };

        let result = SwapResult {
            success: true,
            error: None,
            input_token: from_token.to_string(),
            output_token: to_token.to_string(),
            input_amount,
            output_amount,
            effective_price,
            gas_used: 2_500_000, // Higher gas for two-hop
            execution_time_ms: execution_time,
            ptb_execution,
            balances_after: self.balances.clone(),
        };

        self.swap_history.push(result.clone());

        Ok(result)
    }

    /// Reset session to initial state with fresh orderbook clones
    pub fn reset(&mut self, fresh_orderbooks: HashMap<PoolId, SandboxOrderbook>) {
        self.balances = UserBalances::initial();
        self.swap_history.clear();
        self.orderbooks = fresh_orderbooks;
    }
}

/// Session store for managing multiple trading sessions
pub struct SessionManager {
    sessions: RwLock<HashMap<String, Arc<RwLock<TradingSession>>>>,
    /// Global orderbooks cloned into each new session
    global_orderbooks: RwLock<HashMap<PoolId, SandboxOrderbook>>,
}

impl SessionManager {
    pub fn new(global_orderbooks: HashMap<PoolId, SandboxOrderbook>) -> Self {
        Self {
            sessions: RwLock::new(HashMap::new()),
            global_orderbooks: RwLock::new(global_orderbooks),
        }
    }

    /// Create a new session with cloned orderbooks
    pub async fn create_session(&self) -> Result<String> {
        let session_id = uuid::Uuid::new_v4().to_string();
        let orderbooks = self.global_orderbooks.read().await.clone();
        let session = TradingSession::new(session_id.clone(), orderbooks)?;

        let mut sessions = self.sessions.write().await;
        sessions.insert(session_id.clone(), Arc::new(RwLock::new(session)));

        Ok(session_id)
    }

    /// Get a session by ID
    pub async fn get_session(&self, session_id: &str) -> Option<Arc<RwLock<TradingSession>>> {
        let sessions = self.sessions.read().await;
        sessions.get(session_id).cloned()
    }
}
