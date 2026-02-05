'use client';

import { useState } from 'react';
import { useOrderBook, useSession } from '@/hooks/useSession';

const POOLS = [
  { id: 'sui_usdc', label: 'SUI/USDC', base: 'SUI' },
  { id: 'deep_usdc', label: 'DEEP/USDC', base: 'DEEP' },
  { id: 'wal_usdc', label: 'WAL/USDC', base: 'WAL' },
] as const;

export function OrderBook() {
  const [selectedPool, setSelectedPool] = useState('sui_usdc');
  const { session } = useSession();
  const { orderBook, isLoading } = useOrderBook(selectedPool, session?.session_id);

  const pool = POOLS.find((p) => p.id === selectedPool) ?? POOLS[0];
  const ob = orderBook?.orderbook;
  const asks = ob?.asks ?? [];
  const bids = ob?.bids ?? [];

  // Show top N levels (reversed asks so lowest ask is at bottom near mid price)
  const displayAsks = asks.slice(0, 8).reverse();
  const displayBids = bids.slice(0, 8);

  const maxTotal = Math.max(
    ...displayAsks.map((a) => a.total),
    ...displayBids.map((b) => b.total),
    1
  );

  return (
    <div className="bg-deep-card border border-deep-border rounded-xl p-4">
      {/* Pool Selector Tabs */}
      <div className="flex items-center gap-1 mb-4">
        {POOLS.map((p) => (
          <button
            key={p.id}
            onClick={() => setSelectedPool(p.id)}
            className={`px-3 py-1.5 text-sm rounded-lg transition-colors ${
              selectedPool === p.id
                ? 'bg-deep-blue text-white'
                : 'text-gray-400 hover:text-gray-200 hover:bg-deep-dark'
            }`}
          >
            {p.label}
          </button>
        ))}
      </div>

      {/* Header */}
      <div className="grid grid-cols-3 text-xs text-gray-500 pb-2 border-b border-deep-border">
        <span>Price (USDC)</span>
        <span className="text-right">Qty ({pool.base})</span>
        <span className="text-right">Total (USDC)</span>
      </div>

      {isLoading || !ob ? (
        <div className="py-16 text-center text-gray-500">
          <svg className="animate-spin h-6 w-6 mx-auto mb-2" viewBox="0 0 24 24">
            <circle className="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" strokeWidth="4" fill="none" />
            <path className="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4zm2 5.291A7.962 7.962 0 014 12H0c0 3.042 1.135 5.824 3 7.938l3-2.647z" />
          </svg>
          Loading orderbook...
        </div>
      ) : (
        <>
          {/* Asks (Sells) */}
          <div className="py-2">
            {displayAsks.map((level, i) => (
              <div
                key={`ask-${i}`}
                className="grid grid-cols-3 py-1 text-sm relative"
              >
                <div
                  className="absolute inset-0 ask-bar"
                  style={{ width: `${(level.total / maxTotal) * 100}%`, right: 0, left: 'auto' }}
                />
                <span className="text-ask relative z-10">{level.price.toFixed(4)}</span>
                <span className="text-right relative z-10">{level.quantity.toLocaleString()}</span>
                <span className="text-right text-gray-400 relative z-10">
                  {level.total.toLocaleString(undefined, { maximumFractionDigits: 2 })}
                </span>
              </div>
            ))}
          </div>

          {/* Mid Price */}
          <div className="py-3 border-y border-deep-border flex items-center justify-center gap-2">
            <span className="text-2xl font-mono font-bold">
              {ob.mid_price != null ? ob.mid_price.toFixed(4) : '—'}
            </span>
            <span className="text-gray-500 text-sm">USDC</span>
            {ob.spread_bps != null && (
              <span className="text-xs text-gray-500 ml-2">
                Spread: {ob.spread_bps.toFixed(1)} bps
              </span>
            )}
          </div>

          {/* Bids (Buys) */}
          <div className="py-2">
            {displayBids.map((level, i) => (
              <div
                key={`bid-${i}`}
                className="grid grid-cols-3 py-1 text-sm relative"
              >
                <div
                  className="absolute inset-0 bid-bar"
                  style={{ width: `${(level.total / maxTotal) * 100}%` }}
                />
                <span className="text-bid relative z-10">{level.price.toFixed(4)}</span>
                <span className="text-right relative z-10">{level.quantity.toLocaleString()}</span>
                <span className="text-right text-gray-400 relative z-10">
                  {level.total.toLocaleString(undefined, { maximumFractionDigits: 2 })}
                </span>
              </div>
            ))}
          </div>
        </>
      )}

      {/* Footer */}
      <div className="pt-3 border-t border-deep-border text-xs text-gray-500 text-center">
        Forked from mainnet • Checkpoint 240M
      </div>
    </div>
  );
}
