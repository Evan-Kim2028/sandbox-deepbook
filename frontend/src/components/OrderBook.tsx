'use client';

import { useOrderBook } from '@/hooks/useSession';

interface OrderLevel {
  price: number;
  size: number;
  total: number;
}

export function OrderBook() {
  const { orderBook, isLoading } = useOrderBook();

  // Mock data for now - will be replaced with real forked data
  const mockAsks: OrderLevel[] = [
    { price: 3.98, size: 1250, total: 4975 },
    { price: 3.97, size: 890, total: 3533 },
    { price: 3.96, size: 2100, total: 8316 },
    { price: 3.95, size: 450, total: 1777 },
    { price: 3.94, size: 1800, total: 7092 },
  ].reverse();

  const mockBids: OrderLevel[] = [
    { price: 3.92, size: 1500, total: 5880 },
    { price: 3.91, size: 2200, total: 8602 },
    { price: 3.90, size: 980, total: 3822 },
    { price: 3.89, size: 1650, total: 6418 },
    { price: 3.88, size: 3100, total: 12028 },
  ];

  const maxTotal = Math.max(
    ...mockAsks.map((a) => a.total),
    ...mockBids.map((b) => b.total)
  );

  const midPrice = 3.93;

  return (
    <div className="bg-deep-card border border-deep-border rounded-xl p-4">
      <div className="flex items-center justify-between mb-4">
        <h2 className="text-lg font-semibold">Order Book</h2>
        <span className="text-sm text-gray-500">SUI/USDC</span>
      </div>

      {/* Header */}
      <div className="grid grid-cols-3 text-xs text-gray-500 pb-2 border-b border-deep-border">
        <span>Price (USDC)</span>
        <span className="text-right">Size (SUI)</span>
        <span className="text-right">Total (USDC)</span>
      </div>

      {/* Asks (Sells) */}
      <div className="py-2">
        {mockAsks.map((level, i) => (
          <div
            key={`ask-${i}`}
            className="grid grid-cols-3 py-1 text-sm relative"
          >
            <div
              className="absolute inset-0 ask-bar"
              style={{ width: `${(level.total / maxTotal) * 100}%`, right: 0, left: 'auto' }}
            />
            <span className="text-ask relative z-10">{level.price.toFixed(2)}</span>
            <span className="text-right relative z-10">{level.size.toLocaleString()}</span>
            <span className="text-right text-gray-400 relative z-10">
              {level.total.toLocaleString()}
            </span>
          </div>
        ))}
      </div>

      {/* Mid Price */}
      <div className="py-3 border-y border-deep-border flex items-center justify-center gap-2">
        <span className="text-2xl font-mono font-bold">{midPrice.toFixed(2)}</span>
        <span className="text-gray-500 text-sm">USDC</span>
      </div>

      {/* Bids (Buys) */}
      <div className="py-2">
        {mockBids.map((level, i) => (
          <div
            key={`bid-${i}`}
            className="grid grid-cols-3 py-1 text-sm relative"
          >
            <div
              className="absolute inset-0 bid-bar"
              style={{ width: `${(level.total / maxTotal) * 100}%` }}
            />
            <span className="text-bid relative z-10">{level.price.toFixed(2)}</span>
            <span className="text-right relative z-10">{level.size.toLocaleString()}</span>
            <span className="text-right text-gray-400 relative z-10">
              {level.total.toLocaleString()}
            </span>
          </div>
        ))}
      </div>

      {/* Footer */}
      <div className="pt-3 border-t border-deep-border text-xs text-gray-500 text-center">
        Forked from mainnet • Pool: 0xe05d...4407
      </div>
    </div>
  );
}
