'use client';

import { useEffect, useState } from 'react';
import { toast } from 'sonner';
import { useDebugPoolStatus, useDebugPools, useEnsureDebugPool } from '@/hooks/useSession';

export function DebugPoolPanel() {
  const { debugPool } = useDebugPoolStatus();
  const { pools } = useDebugPools();
  const ensureDebugPool = useEnsureDebugPool();

  const [tokenSymbol, setTokenSymbol] = useState('DBG');
  const [tokenName, setTokenName] = useState('Debug Token');
  const [tokenDescription, setTokenDescription] = useState('Local VM debug token for DeepBook sandbox flows');
  const [tokenIconUrl, setTokenIconUrl] = useState('');
  const [bidPrice, setBidPrice] = useState('0.90');
  const [askPrice, setAskPrice] = useState('1.10');
  const [bidQty, setBidQty] = useState('100');
  const [askQty, setAskQty] = useState('100');
  const [baseLiquidity, setBaseLiquidity] = useState('200');
  const [quoteLiquidity, setQuoteLiquidity] = useState('200');

  useEffect(() => {
    if (!debugPool) return;
    const decimals = 9;
    setTokenSymbol(debugPool.token_symbol);
    setTokenName(debugPool.token_name);
    setTokenDescription(debugPool.token_description);
    setTokenIconUrl(debugPool.token_icon_url);
    setBidPrice((debugPool.config.bid_price / 1_000_000).toFixed(2));
    setAskPrice((debugPool.config.ask_price / 1_000_000).toFixed(2));
    setBidQty((debugPool.config.bid_quantity / Math.pow(10, decimals)).toFixed(2));
    setAskQty((debugPool.config.ask_quantity / Math.pow(10, decimals)).toFixed(2));
    setBaseLiquidity((debugPool.config.base_liquidity / Math.pow(10, decimals)).toFixed(2));
    setQuoteLiquidity((debugPool.config.quote_liquidity / 1_000_000).toFixed(2));
  }, [debugPool]);

  const onSubmit = async () => {
    try {
      const symbol = tokenSymbol.trim().toUpperCase();
      const name = tokenName.trim();
      const description = tokenDescription.trim();
      const iconUrl = tokenIconUrl.trim();
      const decimals = 9;
      if (!symbol) throw new Error('Token symbol is required');
      if (!name) throw new Error('Token name is required');

      const toPositive = (v: string, label: string) => {
        const parsed = Number(v);
        if (!Number.isFinite(parsed) || parsed <= 0) throw new Error(`${label} must be > 0`);
        return parsed;
      };

      await ensureDebugPool.mutateAsync({
        token_symbol: symbol,
        token_name: name,
        token_description: description,
        token_icon_url: iconUrl,
        bid_price: Math.round(toPositive(bidPrice, 'Bid price') * 1_000_000),
        ask_price: Math.round(toPositive(askPrice, 'Ask price') * 1_000_000),
        bid_quantity: Math.round(toPositive(bidQty, 'Bid quantity') * Math.pow(10, decimals)),
        ask_quantity: Math.round(toPositive(askQty, 'Ask quantity') * Math.pow(10, decimals)),
        base_liquidity: Math.round(toPositive(baseLiquidity, 'Base liquidity') * Math.pow(10, decimals)),
        quote_liquidity: Math.round(toPositive(quoteLiquidity, 'Quote liquidity') * 1_000_000),
        whitelisted_pool: true,
        pay_with_deep: false,
      });

      toast.success(`Custom pool ${symbol}/USDC is ready in local VM`);
    } catch (err) {
      toast.error((err as Error).message || 'Failed to create debug pool');
    }
  };

  return (
    <div className="bg-deep-card border border-deep-border rounded-xl p-4 mb-6">
      <div className="flex items-center justify-between mb-3">
        <h2 className="text-lg font-semibold">Token + Pool Creator</h2>
        <span className={`text-xs px-2 py-1 rounded ${debugPool?.created ? 'bg-green-500/20 text-green-400' : 'bg-gray-700 text-gray-300'}`}>
          {debugPool?.created ? 'Created' : 'Not Created'}
        </span>
      </div>

      <p className="text-sm text-gray-400 mb-4">
        VM-native flow: create token metadata, create pool, and seed orderbook liquidity through Move PTBs.
      </p>

      <div className="grid grid-cols-1 md:grid-cols-2 gap-3 mb-3">
        <label className="text-xs text-gray-400">
          Token Symbol
          <input
            value={tokenSymbol}
            onChange={(e) => setTokenSymbol(e.target.value)}
            className="mt-1 w-full bg-deep-dark border border-deep-border rounded px-2 py-1.5 text-sm"
            placeholder="DBG"
          />
        </label>
        <label className="text-xs text-gray-400">
          Token Name
          <input
            value={tokenName}
            onChange={(e) => setTokenName(e.target.value)}
            className="mt-1 w-full bg-deep-dark border border-deep-border rounded px-2 py-1.5 text-sm"
            placeholder="Debug Token"
          />
        </label>
      </div>

      <div className="grid grid-cols-1 md:grid-cols-2 gap-3 mb-4">
        <label className="text-xs text-gray-400">
          Token Description
          <input
            value={tokenDescription}
            onChange={(e) => setTokenDescription(e.target.value)}
            className="mt-1 w-full bg-deep-dark border border-deep-border rounded px-2 py-1.5 text-sm"
            placeholder="Describe token purpose for sandbox run"
          />
        </label>
        <label className="text-xs text-gray-400">
          Token Icon URL (optional)
          <input
            value={tokenIconUrl}
            onChange={(e) => setTokenIconUrl(e.target.value)}
            className="mt-1 w-full bg-deep-dark border border-deep-border rounded px-2 py-1.5 text-sm"
            placeholder="https://..."
          />
        </label>
      </div>

      <div className="grid grid-cols-2 md:grid-cols-4 gap-3">
        <label className="text-xs text-gray-400">
          Bid Price (USDC)
          <input
            type="number"
            value={bidPrice}
            onChange={(e) => setBidPrice(e.target.value)}
            className="mt-1 w-full bg-deep-dark border border-deep-border rounded px-2 py-1.5 text-sm"
          />
        </label>
        <label className="text-xs text-gray-400">
          Ask Price (USDC)
          <input
            type="number"
            value={askPrice}
            onChange={(e) => setAskPrice(e.target.value)}
            className="mt-1 w-full bg-deep-dark border border-deep-border rounded px-2 py-1.5 text-sm"
          />
        </label>
        <label className="text-xs text-gray-400">
          Bid Qty (Token)
          <input
            type="number"
            value={bidQty}
            onChange={(e) => setBidQty(e.target.value)}
            className="mt-1 w-full bg-deep-dark border border-deep-border rounded px-2 py-1.5 text-sm"
          />
        </label>
        <label className="text-xs text-gray-400">
          Ask Qty (Token)
          <input
            type="number"
            value={askQty}
            onChange={(e) => setAskQty(e.target.value)}
            className="mt-1 w-full bg-deep-dark border border-deep-border rounded px-2 py-1.5 text-sm"
          />
        </label>
        <label className="text-xs text-gray-400">
          Base Liquidity
          <input
            type="number"
            value={baseLiquidity}
            onChange={(e) => setBaseLiquidity(e.target.value)}
            className="mt-1 w-full bg-deep-dark border border-deep-border rounded px-2 py-1.5 text-sm"
          />
        </label>
        <label className="text-xs text-gray-400">
          Quote Liquidity (USDC)
          <input
            type="number"
            value={quoteLiquidity}
            onChange={(e) => setQuoteLiquidity(e.target.value)}
            className="mt-1 w-full bg-deep-dark border border-deep-border rounded px-2 py-1.5 text-sm"
          />
        </label>
        <div className="flex items-end">
          <button
            onClick={onSubmit}
            disabled={ensureDebugPool.isPending}
            className="w-full py-2 bg-deep-blue hover:bg-deep-blue/90 disabled:bg-gray-700 rounded font-medium text-sm transition-colors"
          >
            {ensureDebugPool.isPending ? 'Applying...' : 'Create / Ensure'}
          </button>
        </div>
      </div>

      <div className="mt-4 pt-3 border-t border-deep-border">
        <h3 className="text-sm font-semibold mb-2">Created Pools</h3>
        {pools.length === 0 ? (
          <p className="text-xs text-gray-500">No custom pool created yet.</p>
        ) : (
          <div className="space-y-2">
            {pools.map((pool) => (
              <div key={pool.pool_object_id ?? pool.token_symbol} className="text-xs bg-deep-dark border border-deep-border rounded p-2">
                <div className="text-gray-300">
                  {pool.token_symbol}/USDC • {pool.token_name}
                </div>
                <div className="text-gray-500 mt-1 break-all">
                  pool: {pool.pool_object_id ?? 'pending'}
                </div>
              </div>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}
