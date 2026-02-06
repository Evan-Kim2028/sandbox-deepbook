'use client';

import { useEffect, useRef } from 'react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { api } from '@/lib/api';
import type { OrderBookResponse, QuoteResponse } from '@/types';

const SESSION_STORAGE_KEY = 'deepbook_sandbox_session_id';

export function useSession() {
  const resolveSession = async () => {
    const storedId =
      typeof window !== 'undefined'
        ? window.localStorage.getItem(SESSION_STORAGE_KEY)
        : null;

    if (storedId) {
      try {
        return await api.getSession(storedId);
      } catch {
        if (typeof window !== 'undefined') {
          window.localStorage.removeItem(SESSION_STORAGE_KEY);
        }
      }
    }

    const created = await api.createSession();
    if (typeof window !== 'undefined') {
      window.localStorage.setItem(SESSION_STORAGE_KEY, created.session_id);
    }
    return created;
  };

  const { data: session, isLoading } = useQuery({
    queryKey: ['session'],
    queryFn: resolveSession,
    staleTime: Infinity,
    retry: 3,
  });

  useEffect(() => {
    if (!session?.session_id || typeof window === 'undefined') return;
    window.localStorage.setItem(SESSION_STORAGE_KEY, session.session_id);
  }, [session?.session_id]);

  return { session, isLoading };
}

export function useBalances() {
  const { session } = useSession();

  const { data: balances, isLoading } = useQuery({
    queryKey: ['balances', session?.session_id],
    queryFn: () => api.getBalances(session!.session_id),
    enabled: !!session?.session_id,
    refetchInterval: 5000,
  });

  return { balances, isLoading };
}

export function useOrderBook(pool: string = 'sui_usdc', sessionId?: string) {
  const { data: orderBook, isLoading } = useQuery<OrderBookResponse>({
    queryKey: ['orderbook', pool, sessionId],
    queryFn: () => api.getOrderBook(pool, sessionId),
    refetchInterval: 10000,
  });

  return { orderBook, isLoading };
}

export function useQuote(fromToken: string, toToken: string, amount: string, sessionId?: string) {
  const debounceRef = useRef<ReturnType<typeof setTimeout>>();
  const queryClient = useQueryClient();

  const hasAmount = !!amount && parseFloat(amount) > 0;
  const queryKey = ['quote', fromToken, toToken, amount, sessionId];

  const { data: quote, isLoading, isFetching } = useQuery<QuoteResponse>({
    queryKey,
    queryFn: () =>
      api.getQuote({
        from_token: fromToken.toLowerCase(),
        to_token: toToken.toLowerCase(),
        amount,
        session_id: sessionId,
      }),
    enabled: hasAmount,
    staleTime: 5000,
    retry: false,
  });

  // Debounce: invalidate query after 500ms of no changes
  useEffect(() => {
    if (!hasAmount) return;
    if (debounceRef.current) clearTimeout(debounceRef.current);
    debounceRef.current = setTimeout(() => {
      queryClient.invalidateQueries({ queryKey });
    }, 500);
    return () => {
      if (debounceRef.current) clearTimeout(debounceRef.current);
    };
  }, [fromToken, toToken, amount]);

  return { quote: hasAmount ? quote : undefined, isLoading: isFetching && hasAmount };
}

export function usePools() {
  const { data, isLoading } = useQuery({
    queryKey: ['pools'],
    queryFn: api.getPools,
    staleTime: 30000,
  });

  return { pools: data?.pools, totalLoaded: data?.total_loaded, isLoading };
}

export function useDebugPoolStatus() {
  const { data, isLoading } = useQuery({
    queryKey: ['debug-pool'],
    queryFn: api.getDebugPoolStatus,
    staleTime: 15000,
    refetchInterval: 30000,
  });

  return { debugPool: data, isLoading };
}

export function useDebugPools() {
  const { data, isLoading } = useQuery({
    queryKey: ['debug-pools'],
    queryFn: api.getDebugPools,
    staleTime: 10000,
    refetchInterval: 30000,
  });

  return { pools: data?.pools ?? [], isLoading };
}

export function useEnsureDebugPool() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: api.ensureDebugPool,
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['debug-pool'] });
      queryClient.invalidateQueries({ queryKey: ['debug-pools'] });
      queryClient.invalidateQueries({ queryKey: ['balances'] });
      queryClient.invalidateQueries({ queryKey: ['quote'] });
      queryClient.invalidateQueries({ queryKey: ['orderbook'] });
    },
  });
}

export function useStartupCheck() {
  const { data, isLoading } = useQuery({
    queryKey: ['startup-check'],
    queryFn: api.getStartupCheck,
    staleTime: 15000,
    refetchInterval: 30000,
  });

  return { startupCheck: data, isLoading };
}

export function useFaucet() {
  const queryClient = useQueryClient();
  const { session } = useSession();

  return useMutation({
    mutationFn: async (params: { token: string; amount: string }) => {
      if (!session) throw new Error('No session');

      return api.requestTokens({
        session_id: session.session_id,
        token: params.token,
        amount: params.amount,
      });
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['balances'] });
    },
  });
}
