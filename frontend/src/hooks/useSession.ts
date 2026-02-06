'use client';

import { useEffect, useRef } from 'react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { api } from '@/lib/api';
import type { Balances, OrderBookResponse, QuoteResponse } from '@/types';

export function useSession() {
  const { data: session, isLoading } = useQuery({
    queryKey: ['session'],
    queryFn: api.createSession,
    staleTime: Infinity,
    retry: 3,
  });

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

export function useFaucet() {
  const queryClient = useQueryClient();
  const { session } = useSession();

  return useMutation({
    mutationFn: async (token: 'sui' | 'usdc' | 'wal' | 'deep') => {
      if (!session) throw new Error('No session');

      const amounts: Record<string, string> = {
        sui: '10000000000',   // 10 SUI
        usdc: '100000000',    // 100 USDC
        wal: '50000000000',   // 50 WAL
        deep: '100000000', // 100 DEEP
      };

      return api.requestTokens({
        session_id: session.session_id,
        token,
        amount: amounts[token],
      });
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['balances'] });
    },
  });
}
