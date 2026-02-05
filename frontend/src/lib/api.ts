import type {
  Session,
  Balances,
  SwapRequest,
  SwapResponse,
  FaucetRequest,
  FaucetResponse,
  OrderBookResponse,
  QuoteResponse,
  PoolInfo,
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
    fetchAPI<{ session_id: string; balances: Balances }>(`/balance/${sessionId}`).then(
      (r) => r.balances
    ),

  // Pools
  getPools: () =>
    fetchAPI<{ total_loaded: number; pools: PoolInfo[] }>('/pools'),

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

  getQuote: (params: { from_token: string; to_token: string; amount: string; session_id?: string }) =>
    fetchAPI<QuoteResponse>('/swap/quote', {
      method: 'POST',
      body: JSON.stringify(params),
    }),

  // Order Book
  getOrderBook: (pool?: string, sessionId?: string) => {
    const params = new URLSearchParams();
    if (pool) params.set('pool', pool);
    if (sessionId) params.set('session_id', sessionId);
    const qs = params.toString();
    return fetchAPI<OrderBookResponse>(`/orderbook${qs ? `?${qs}` : ''}`);
  },
};
