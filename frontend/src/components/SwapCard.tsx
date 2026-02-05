'use client';

import { useState } from 'react';
import { useMutation, useQueryClient } from '@tanstack/react-query';
import { toast } from 'sonner';
import { api } from '@/lib/api';
import { formatBalance } from '@/lib/utils';
import { useActivityStore } from '@/hooks/useActivityStore';

interface SwapCardProps {
  sessionId?: string;
}

const TOKENS = {
  SUI: { symbol: 'SUI', decimals: 9, icon: '◎' },
  USDC: { symbol: 'USDC', decimals: 6, icon: '$' },
};

export function SwapCard({ sessionId }: SwapCardProps) {
  const [fromToken, setFromToken] = useState<'SUI' | 'USDC'>('USDC');
  const [toToken, setToToken] = useState<'SUI' | 'USDC'>('SUI');
  const [amount, setAmount] = useState('');
  const queryClient = useQueryClient();
  const addActivity = useActivityStore((s) => s.addActivity);

  // Mock price for now
  const price = 3.92; // USDC per SUI
  const estimatedOutput = fromToken === 'USDC'
    ? (parseFloat(amount || '0') / price).toFixed(4)
    : (parseFloat(amount || '0') * price).toFixed(2);

  const swapMutation = useMutation({
    mutationFn: async () => {
      if (!sessionId) throw new Error('No session');
      return api.executeSwap({
        session_id: sessionId,
        from_token: fromToken.toLowerCase(),
        to_token: toToken.toLowerCase(),
        amount: (parseFloat(amount) * Math.pow(10, TOKENS[fromToken].decimals)).toString(),
      });
    },
    onSuccess: (data) => {
      const outputFormatted = formatBalance(data.output_amount, TOKENS[toToken].decimals);

      // Add to activity feed
      addActivity({
        type: 'swap',
        description: `Swapped ${amount} ${fromToken} → ${outputFormatted} ${toToken}`,
        execution_time_ms: data.execution_time_ms,
        gas_used: data.gas_used,
        ptb_details: data.ptb_details,
      });

      toast.success(
        <div>
          <p className="font-medium">Swap Executed!</p>
          <p className="text-sm text-gray-400">
            {amount} {fromToken} → {outputFormatted} {toToken}
          </p>
          <p className="text-xs text-gray-500 mt-1">
            {data.execution_time_ms}ms • Gas: {parseInt(data.gas_used).toLocaleString()}
          </p>
        </div>
      );
      setAmount('');
      queryClient.invalidateQueries({ queryKey: ['balances'] });
    },
    onError: (error) => {
      toast.error('Swap failed: ' + (error as Error).message);
    },
  });

  const handleSwapTokens = () => {
    setFromToken(toToken);
    setToToken(fromToken);
    setAmount('');
  };

  return (
    <div className="bg-deep-card border border-deep-border rounded-xl p-4">
      <h2 className="text-lg font-semibold mb-4">Swap</h2>

      {/* From Token */}
      <div className="bg-deep-dark rounded-lg p-4 mb-2">
        <div className="flex justify-between mb-2">
          <span className="text-sm text-gray-500">From</span>
          <span className="text-sm text-gray-500">Balance: --</span>
        </div>
        <div className="flex items-center gap-3">
          <input
            type="number"
            value={amount}
            onChange={(e) => setAmount(e.target.value)}
            placeholder="0.00"
            className="flex-1 bg-transparent text-2xl font-mono outline-none"
          />
          <button className="flex items-center gap-2 px-3 py-2 bg-deep-card rounded-lg border border-deep-border">
            <span>{TOKENS[fromToken].icon}</span>
            <span className="font-medium">{fromToken}</span>
          </button>
        </div>
      </div>

      {/* Swap Direction Button */}
      <div className="flex justify-center -my-1 relative z-10">
        <button
          onClick={handleSwapTokens}
          className="w-10 h-10 bg-deep-card border border-deep-border rounded-lg flex items-center justify-center hover:bg-deep-border transition-colors"
        >
          <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M7 16V4m0 0L3 8m4-4l4 4m6 0v12m0 0l4-4m-4 4l-4-4" />
          </svg>
        </button>
      </div>

      {/* To Token */}
      <div className="bg-deep-dark rounded-lg p-4 mt-2">
        <div className="flex justify-between mb-2">
          <span className="text-sm text-gray-500">To</span>
          <span className="text-sm text-gray-500">Balance: --</span>
        </div>
        <div className="flex items-center gap-3">
          <input
            type="text"
            value={amount ? estimatedOutput : ''}
            placeholder="0.00"
            readOnly
            className="flex-1 bg-transparent text-2xl font-mono outline-none text-gray-400"
          />
          <button className="flex items-center gap-2 px-3 py-2 bg-deep-card rounded-lg border border-deep-border">
            <span>{TOKENS[toToken].icon}</span>
            <span className="font-medium">{toToken}</span>
          </button>
        </div>
      </div>

      {/* Price Info */}
      {amount && (
        <div className="mt-4 p-3 bg-deep-dark rounded-lg text-sm">
          <div className="flex justify-between text-gray-400">
            <span>Price</span>
            <span>1 SUI = {price} USDC</span>
          </div>
          <div className="flex justify-between text-gray-400 mt-1">
            <span>Price Impact</span>
            <span className="text-green-400">{'<0.01%'}</span>
          </div>
        </div>
      )}

      {/* Swap Button */}
      <button
        onClick={() => swapMutation.mutate()}
        disabled={!amount || !sessionId || swapMutation.isPending}
        className="w-full mt-4 py-4 bg-deep-blue hover:bg-deep-blue/90 disabled:bg-gray-700 disabled:cursor-not-allowed rounded-xl font-semibold text-lg transition-colors"
      >
        {swapMutation.isPending ? (
          <span className="flex items-center justify-center gap-2">
            <svg className="animate-spin h-5 w-5" viewBox="0 0 24 24">
              <circle className="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" strokeWidth="4" fill="none" />
              <path className="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4zm2 5.291A7.962 7.962 0 014 12H0c0 3.042 1.135 5.824 3 7.938l3-2.647z" />
            </svg>
            Executing...
          </span>
        ) : (
          'Swap'
        )}
      </button>

      {/* Sandbox Note */}
      <p className="text-center text-xs text-gray-500 mt-3">
        Simulated execution • No gas fees
      </p>
    </div>
  );
}
