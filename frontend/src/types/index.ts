export interface Session {
  session_id: string;
  created_at: number;
  expires_at: number;
  checkpoint: number;
  balances: Balances;
}

export interface Balances {
  sui: string;
  sui_human: number;
  usdc: string;
  usdc_human: number;
  deep: string;
  deep_human: number;
  wal: string;
  wal_human: number;
  custom?: Record<string, string>;
}

export interface SwapRequest {
  session_id: string;
  from_token: string;
  to_token: string;
  amount: string;
  slippage_bps?: number;
}

export interface SwapResponse {
  success: boolean;
  input_token: string;
  output_token: string;
  input_amount: string;
  input_amount_human: number;
  output_amount: string;
  output_amount_human: number;
  effective_price: number;
  price_impact_bps: number;
  execution_method: string;
  message: string;
  gas_used?: string;
  execution_time_ms?: number;
  ptb_execution?: PtbExecution;
  balances_after?: Balances;
  route_type?: 'direct' | 'two_hop';
  intermediate_amount?: number;
}

export interface PtbExecution {
  commands: PtbCommand[];
  status: string;
  effects_digest: string | null;
  events: PtbEvent[];
  summary: string;
}

export interface PtbCommand {
  index: number;
  command_type: string;
  description: string;
  package?: string;
  module?: string;
  function?: string;
  type_args?: string[];
}

export interface PtbEvent {
  event_type: string;
  data: Record<string, unknown>;
}

export interface FaucetRequest {
  session_id: string;
  token: string;
  amount: string;
}

export interface FaucetResponse {
  success: boolean;
  new_balance: string;
  new_balance_human?: number;
  token: string;
}

export interface OrderBookLevel {
  price: number;
  quantity: number;
  total: number;
  orders: number;
}

export interface OrderBookSnapshot {
  pool_id: string;
  base_symbol: string;
  quote_symbol: string;
  mid_price: number | null;
  best_bid: number | null;
  best_ask: number | null;
  spread_bps: number | null;
  bids: OrderBookLevel[];
  asks: OrderBookLevel[];
  timestamp: number;
}

export interface OrderBookStats {
  total_bids: number;
  total_asks: number;
  total_bid_volume: number;
  total_ask_volume: number;
}

export interface OrderBookResponse {
  success: boolean;
  error?: string;
  orderbook?: OrderBookSnapshot;
  stats?: OrderBookStats;
}

export interface QuoteResponse {
  success: boolean;
  error?: string;
  pool: string;
  input_token: string;
  output_token: string;
  input_amount: string;
  input_amount_human: number;
  estimated_output: string;
  estimated_output_human: number;
  effective_price: number;
  mid_price: number;
  price_impact_bps: number;
  levels_consumed: number;
  orders_matched: number;
  fully_fillable: boolean;
  route: string;
  route_type?: 'direct' | 'two_hop';
  intermediate_amount?: number;
}

export interface PoolInfo {
  pool_id: string;
  display_name: string;
  loaded: boolean;
  orderbook_ready: boolean;
  mid_price?: number;
  bid_levels?: number;
  ask_levels?: number;
}

export interface DebugPoolConfig {
  tick_size: number;
  lot_size: number;
  min_size: number;
  whitelisted_pool: boolean;
  pay_with_deep: boolean;
  bid_price: number;
  ask_price: number;
  bid_quantity: number;
  ask_quantity: number;
  base_liquidity: number;
  quote_liquidity: number;
  deep_fee_budget: number;
}

export interface DebugPoolStatus {
  success: boolean;
  created: boolean;
  pool_object_id?: string;
  token_symbol: string;
  token_name: string;
  token_description: string;
  token_icon_url: string;
  token_decimals: number;
  token_type: string;
  config: DebugPoolConfig;
  message?: string;
}

export interface DebugPoolListResponse {
  success: boolean;
  pools: DebugPoolStatus[];
}

export interface EnsureDebugPoolRequest {
  token_symbol?: string;
  token_name?: string;
  token_description?: string;
  token_icon_url?: string;
  tick_size?: number;
  lot_size?: number;
  min_size?: number;
  whitelisted_pool?: boolean;
  pay_with_deep?: boolean;
  bid_price?: number;
  ask_price?: number;
  bid_quantity?: number;
  ask_quantity?: number;
  base_liquidity?: number;
  quote_liquidity?: number;
  deep_fee_budget?: number;
}

export interface RouterSharedObjectCheck {
  name: string;
  object_id: string;
  present: boolean;
  is_shared: boolean;
  version?: number;
}

export interface RouterReserveCoinCheck {
  coin_type: string;
  object_id?: string;
  present: boolean;
  version?: number;
  value?: number;
}

export interface StartupCheckReport {
  ok: boolean;
  checked_at_unix_ms: number;
  router_package_deployed: boolean;
  router_health_check_passed: boolean;
  shared_objects: RouterSharedObjectCheck[];
  reserve_coins: RouterReserveCoinCheck[];
  errors: string[];
}

export interface SwapMeta {
  input_token: string;
  output_token: string;
  input_amount_human: number;
  output_amount_human: number;
  effective_price: number;
  price_impact_bps: number;
  route_type: 'direct' | 'two_hop';
  intermediate_amount?: number;
  route: string;
}

export interface Activity {
  id: string;
  type: 'swap' | 'faucet';
  description: string;
  timestamp: Date;
  execution_time_ms?: number;
  gas_used?: string;
  ptb_execution?: PtbExecution;
  swapMeta?: SwapMeta;
}
