'use client';

import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { api } from '@/lib/api';
import type { Balances, OrderBook } from '@/types';

export function useSession() {
  const { data: session, isLoading } = useQuery({
    queryKey: ['session'],
    queryFn: api.createSession,
    staleTime: Infinity, // Session doesn't change
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
    refetchInterval: 5000, // Refresh every 5s
  });

  return { balances, isLoading };
}

export function useOrderBook() {
  const { data: orderBook, isLoading } = useQuery({
    queryKey: ['orderbook'],
    queryFn: () => api.getOrderBook(),
    refetchInterval: 10000, // Refresh every 10s
  });

  return { orderBook, isLoading };
}

export function useFaucet() {
  const queryClient = useQueryClient();
  const { session } = useSession();

  return useMutation({
    mutationFn: async (token: 'sui' | 'usdc' | 'wal') => {
      if (!session) throw new Error('No session');

      // Request amount based on token
      const amounts: Record<string, string> = {
        sui: '10000000000', // 10 SUI
        usdc: '100000000',  // 100 USDC
        wal: '50000000000', // 50 WAL
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
