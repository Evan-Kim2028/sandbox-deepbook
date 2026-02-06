//! DeepBook Swap Executor for Session-Based Trading
//!
//! Manages trading sessions with user balances and swap execution.
//! Applies results from VM-executed PTBs to per-session balances/history.

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use super::orderbook_builder::SandboxOrderbook;
use super::state_loader::PoolId;

// Sessions start unfunded; balances are added via VM faucet PTBs.
const INITIAL_SUI: u64 = 0;
const INITIAL_USDC: u64 = 0;
const INITIAL_DEEP: u64 = 0;
const INITIAL_WAL: u64 = 0;

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
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub custom: HashMap<String, u64>,
}

impl UserBalances {
    pub fn initial() -> Self {
        Self {
            sui: INITIAL_SUI,
            usdc: INITIAL_USDC,
            deep: INITIAL_DEEP,
            wal: INITIAL_WAL,
            custom: HashMap::new(),
        }
    }

    pub fn get(&self, token: &str) -> u64 {
        match token.to_uppercase().as_str() {
            "SUI" => self.sui,
            "USDC" => self.usdc,
            "DEEP" => self.deep,
            "WAL" => self.wal,
            other => *self.custom.get(other).unwrap_or(&0),
        }
    }

    pub fn set(&mut self, token: &str, amount: u64) {
        match token.to_uppercase().as_str() {
            "SUI" => self.sui = amount,
            "USDC" => self.usdc = amount,
            "DEEP" => self.deep = amount,
            "WAL" => self.wal = amount,
            other => {
                if amount == 0 {
                    self.custom.remove(other);
                } else {
                    self.custom.insert(other.to_string(), amount);
                }
            }
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

    /// Apply a VM-executed swap to session balances and record it in history.
    ///
    /// `input_amount` is the requested input size, while `input_refund` is the
    /// amount returned by the VM from the input coin.
    pub fn apply_vm_swap(
        &mut self,
        from_token: &str,
        to_token: &str,
        input_amount: u64,
        input_refund: u64,
        deep_input_amount: u64,
        deep_refund: u64,
        output_amount: u64,
        effective_price: f64,
        gas_used: u64,
        execution_time_ms: u64,
        ptb_execution: PtbExecution,
    ) -> Result<SwapResult> {
        if input_refund > input_amount {
            return Err(anyhow!(
                "Invalid VM swap result: input refund {} exceeds input {}",
                input_refund,
                input_amount
            ));
        }
        if deep_refund > deep_input_amount {
            return Err(anyhow!(
                "Invalid VM swap result: deep refund {} exceeds deep input {}",
                deep_refund,
                deep_input_amount
            ));
        }

        if self.balances.get(from_token) < input_amount {
            return Err(anyhow!(
                "Insufficient {} balance: have {}, need {}",
                from_token,
                self.balances.get(from_token),
                input_amount
            ));
        }
        if self.balances.deep < deep_input_amount {
            return Err(anyhow!(
                "Insufficient DEEP balance: have {}, need {}",
                self.balances.deep,
                deep_input_amount
            ));
        }

        let consumed_input = input_amount - input_refund;
        let consumed_deep = deep_input_amount - deep_refund;
        self.balances.subtract(from_token, consumed_input)?;
        self.balances.subtract("DEEP", consumed_deep)?;
        self.balances.add(to_token, output_amount);

        let result = SwapResult {
            success: true,
            error: None,
            input_token: from_token.to_string(),
            output_token: to_token.to_string(),
            input_amount,
            output_amount,
            effective_price,
            gas_used,
            execution_time_ms,
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
