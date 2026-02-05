export interface Session {
  session_id: string;
  created_at: number;
  expires_at: number;
}

export interface Balances {
  sui: string;
  usdc: string;
  wal: string;
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
  input_amount: string;
  output_amount: string;
  from_token: string;
  to_token: string;
  gas_used: string;
  execution_time_ms: number;
  ptb_details: PTBDetails;
}

export interface PTBDetails {
  commands: PTBCommand[];
  gas_budget: string;
  effects_summary: string;
}

export interface PTBCommand {
  index: number;
  command_type: string;
  description: string;
}

export interface FaucetRequest {
  session_id: string;
  token: 'sui' | 'usdc' | 'wal';
  amount: string;
}

export interface FaucetResponse {
  success: boolean;
  new_balance: string;
  token: string;
  ptb_details: PTBDetails;
  execution_time_ms: number;
}

export interface OrderBookLevel {
  price: number;
  size: number;
  total: number;
}

export interface OrderBook {
  asks: OrderBookLevel[];
  bids: OrderBookLevel[];
  mid_price: number;
}

export interface Activity {
  id: string;
  type: 'swap' | 'faucet';
  description: string;
  timestamp: Date;
  execution_time_ms: number;
  gas_used: string;
  ptb_details: PTBDetails;
}
