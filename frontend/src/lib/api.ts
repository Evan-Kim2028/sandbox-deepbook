import type {
  Session,
  Balances,
  SwapRequest,
  SwapResponse,
  FaucetRequest,
  FaucetResponse,
  OrderBook,
} from '@/types';

const API_BASE = '/api';

async function fetchAPI<T>(
  endpoint: string,
  options?: RequestInit
): Promise<T> {
  const res = await fetch(`${API_BASE}${endpoint}`, {
    ...options,
    headers: {
      'Content-Type': 'application/json',
      ...options?.headers,
    },
  });

  if (!res.ok) {
    const error = await res.json().catch(() => ({ error: 'Request failed' }));
    throw new Error(error.error || 'Request failed');
  }

  return res.json();
}

export const api = {
  // Session
  createSession: () =>
    fetchAPI<Session>('/session', { method: 'POST', body: '{}' }),

  getSession: (id: string) => fetchAPI<Session>(`/session/${id}`),

  // Balances
  getBalances: (sessionId: string) =>
    fetchAPI<{ balances: Balances }>(`/balance/${sessionId}`).then(
      (r) => r.balances
    ),

  // Faucet
  requestTokens: (params: FaucetRequest) =>
    fetchAPI<FaucetResponse>('/faucet', {
      method: 'POST',
      body: JSON.stringify(params),
    }),

  // Swap
  executeSwap: (params: SwapRequest) =>
    fetchAPI<SwapResponse>('/swap', {
      method: 'POST',
      body: JSON.stringify(params),
    }),

  getQuote: (params: { from_token: string; to_token: string; amount: string }) =>
    fetchAPI<{ estimated_output: string; price_impact_bps: number }>(
      '/swap/quote',
      {
        method: 'POST',
        body: JSON.stringify(params),
      }
    ),

  // Order Book
  getOrderBook: (poolId?: string) =>
    fetchAPI<OrderBook>(`/orderbook${poolId ? `?pool=${poolId}` : ''}`),
};
