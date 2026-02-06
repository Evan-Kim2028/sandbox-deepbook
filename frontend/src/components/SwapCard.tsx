'use client';

import { useState } from 'react';
import { useMutation, useQueryClient } from '@tanstack/react-query';
import { toast } from 'sonner';
import { api } from '@/lib/api';
import { useBalances, useQuote } from '@/hooks/useSession';
import { useActivityStore } from '@/hooks/useActivityStore';

interface SwapCardProps {
  sessionId?: string;
}

type TokenKey = 'SUI' | 'USDC' | 'DEEP' | 'WAL';

const TOKENS: Record<TokenKey, { symbol: string; decimals: number; icon: string }> = {
  SUI: { symbol: 'SUI', decimals: 9, icon: '◎' },
  USDC: { symbol: 'USDC', decimals: 6, icon: '$' },
  DEEP: { symbol: 'DEEP', decimals: 6, icon: 'D' },
  WAL: { symbol: 'WAL', decimals: 9, icon: 'W' },
};

const TOKEN_KEYS: TokenKey[] = ['SUI', 'USDC', 'DEEP', 'WAL'];

function getBalanceHuman(balances: Record<string, number> | undefined, token: TokenKey): string {
  if (!balances) return '--';
  const key = `${token.toLowerCase()}_human` as keyof typeof balances;
  const val = balances[key];
  if (val == null) return '--';
  return Number(val).toLocaleString(undefined, { minimumFractionDigits: 2, maximumFractionDigits: 2 });
}

export function SwapCard({ sessionId }: SwapCardProps) {
  const [fromToken, setFromToken] = useState<TokenKey>('USDC');
  const [toToken, setToToken] = useState<TokenKey>('SUI');
  const [amount, setAmount] = useState('');
  const [showFromSelect, setShowFromSelect] = useState(false);
  const [showToSelect, setShowToSelect] = useState(false);
  const queryClient = useQueryClient();
  const addActivity = useActivityStore((s) => s.addActivity);
  const { balances } = useBalances();

  // Raw amount in base units for the quote
  const rawAmount = amount && parseFloat(amount) > 0
    ? (parseFloat(amount) * Math.pow(10, TOKENS[fromToken].decimals)).toFixed(0)
    : '';

  const { quote, isLoading: quoteLoading } = useQuote(fromToken, toToken, rawAmount, sessionId);

  const swapMutation = useMutation({
    mutationFn: async () => {
      if (!sessionId) throw new Error('No session');
      return api.executeSwap({
        session_id: sessionId,
        from_token: fromToken.toLowerCase(),
        to_token: toToken.toLowerCase(),
        amount: rawAmount,
      });
    },
    onSuccess: (data) => {
      addActivity({
        type: 'swap',
        description: `Swapped ${data.input_amount_human} ${data.input_token.toUpperCase()} → ${data.output_amount_human} ${data.output_token.toUpperCase()}`,
        execution_time_ms: data.execution_time_ms,
        gas_used: data.gas_used,
        ptb_execution: data.ptb_execution,
        swapMeta: {
          input_token: data.input_token,
          output_token: data.output_token,
          input_amount_human: data.input_amount_human,
          output_amount_human: data.output_amount_human,
          effective_price: data.effective_price,
          price_impact_bps: data.price_impact_bps,
          route_type: data.route_type ?? 'direct',
          intermediate_amount: data.intermediate_amount,
          route: quote?.route ?? '',
        },
      });

      toast.success(
        <div>
          <p className="font-medium">Swap Executed!</p>
          <p className="text-sm text-gray-400">
            {data.input_amount_human} {data.input_token.toUpperCase()} → {data.output_amount_human} {data.output_token.toUpperCase()}
          </p>
          {data.price_impact_bps > 0 && (
            <p className="text-xs text-gray-500 mt-1">
              Price impact: {data.price_impact_bps.toFixed(1)} bps
            </p>
          )}
        </div>
      );
      setAmount('');
      queryClient.invalidateQueries({ queryKey: ['balances'] });
      queryClient.invalidateQueries({ queryKey: ['orderbook'] });
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

  const selectFrom = (t: TokenKey) => {
    if (t === toToken) setToToken(fromToken);
    setFromToken(t);
    setShowFromSelect(false);
    setAmount('');
  };

  const selectTo = (t: TokenKey) => {
    if (t === fromToken) setFromToken(toToken);
    setToToken(t);
    setShowToSelect(false);
    setAmount('');
  };

  return (
    <div className="bg-deep-card border border-deep-border rounded-xl p-4">
      <h2 className="text-lg font-semibold mb-4">Swap</h2>

      {/* From Token */}
      <div className="bg-deep-dark rounded-lg p-4 mb-2">
        <div className="flex justify-between mb-2">
          <span className="text-sm text-gray-500">From</span>
          <span className="text-sm text-gray-500">
            Balance: {getBalanceHuman(balances as unknown as Record<string, number>, fromToken)}
          </span>
        </div>
        <div className="flex items-center gap-3">
          <input
            type="number"
            value={amount}
            onChange={(e) => setAmount(e.target.value)}
            placeholder="0.00"
            className="flex-1 bg-transparent text-2xl font-mono outline-none"
          />
          <div className="relative">
            <button
              onClick={() => setShowFromSelect(!showFromSelect)}
              className="flex items-center gap-2 px-3 py-2 bg-deep-card rounded-lg border border-deep-border"
            >
              <span>{TOKENS[fromToken].icon}</span>
              <span className="font-medium">{fromToken}</span>
              <svg className="w-3 h-3 text-gray-400" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M19 9l-7 7-7-7" />
              </svg>
            </button>
            {showFromSelect && (
              <div className="absolute right-0 mt-1 bg-deep-card border border-deep-border rounded-lg overflow-hidden z-20 min-w-[120px]">
                {TOKEN_KEYS.map((t) => (
                  <button
                    key={t}
                    onClick={() => selectFrom(t)}
                    className={`w-full px-3 py-2 text-left text-sm hover:bg-deep-dark flex items-center gap-2 ${
                      t === fromToken ? 'text-deep-blue' : ''
                    }`}
                  >
                    <span>{TOKENS[t].icon}</span>
                    <span>{t}</span>
                  </button>
                ))}
              </div>
            )}
          </div>
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
          <span className="text-sm text-gray-500">
            Balance: {getBalanceHuman(balances as unknown as Record<string, number>, toToken)}
          </span>
        </div>
        <div className="flex items-center gap-3">
          <input
            type="text"
            value={quote ? quote.estimated_output_human.toLocaleString(undefined, { maximumFractionDigits: 6 }) : ''}
            placeholder={quoteLoading ? 'Loading...' : '0.00'}
            readOnly
            className="flex-1 bg-transparent text-2xl font-mono outline-none text-gray-400"
          />
          <div className="relative">
            <button
              onClick={() => setShowToSelect(!showToSelect)}
              className="flex items-center gap-2 px-3 py-2 bg-deep-card rounded-lg border border-deep-border"
            >
              <span>{TOKENS[toToken].icon}</span>
              <span className="font-medium">{toToken}</span>
              <svg className="w-3 h-3 text-gray-400" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M19 9l-7 7-7-7" />
              </svg>
            </button>
            {showToSelect && (
              <div className="absolute right-0 mt-1 bg-deep-card border border-deep-border rounded-lg overflow-hidden z-20 min-w-[120px]">
                {TOKEN_KEYS.map((t) => (
                  <button
                    key={t}
                    onClick={() => selectTo(t)}
                    className={`w-full px-3 py-2 text-left text-sm hover:bg-deep-dark flex items-center gap-2 ${
                      t === toToken ? 'text-deep-blue' : ''
                    }`}
                  >
                    <span>{TOKENS[t].icon}</span>
                    <span>{t}</span>
                  </button>
                ))}
              </div>
            )}
          </div>
        </div>
      </div>

      {/* Price Info */}
      {quote && (
        <div className="mt-4 p-3 bg-deep-dark rounded-lg text-sm">
          <div className="flex justify-between text-gray-400">
            <span>Effective Price</span>
            <span>1 {quote.input_token.toUpperCase()} = {quote.effective_price.toFixed(6)} {quote.output_token.toUpperCase()}</span>
          </div>
          <div className="flex justify-between text-gray-400 mt-1">
            <span>Price Impact</span>
            <span className={quote.price_impact_bps > 50 ? 'text-yellow-400' : 'text-green-400'}>
              {quote.price_impact_bps.toFixed(1)} bps
            </span>
          </div>
          <div className="flex justify-between text-gray-400 mt-1">
            <span>Route</span>
            {quote.route_type === 'two_hop' ? (
              <span className="flex items-center gap-1">
                <span className="text-white font-medium">{quote.input_token.toUpperCase()}</span>
                <span className="text-gray-500">&rarr;</span>
                <span className="text-green-400 font-medium">USDC</span>
                <span className="text-gray-500">&rarr;</span>
                <span className="text-white font-medium">{quote.output_token.toUpperCase()}</span>
              </span>
            ) : (
              <span>{quote.route}</span>
            )}
          </div>
          {quote.route_type === 'two_hop' && quote.intermediate_amount != null && (
            <div className="flex justify-between text-gray-400 mt-1">
              <span>Intermediate</span>
              <span className="text-green-400">{quote.intermediate_amount.toFixed(2)} USDC</span>
            </div>
          )}
          {!quote.fully_fillable && (
            <div className="mt-2 text-xs text-yellow-400">
              Partial fill — not enough liquidity for full amount
            </div>
          )}
        </div>
      )}

      {/* Swap Button */}
      <button
        onClick={() => swapMutation.mutate()}
        disabled={!amount || !sessionId || swapMutation.isPending || !quote}
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
