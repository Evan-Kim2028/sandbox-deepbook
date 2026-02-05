'use client';

import { useState } from 'react';
import { useBalances, useSession } from '@/hooks/useSession';
import { FaucetModal } from './FaucetModal';

export function Header() {
  const { balances } = useBalances();
  const { session } = useSession();
  const [showFaucet, setShowFaucet] = useState(false);

  const fmt = (val: number | undefined) =>
    val != null
      ? val.toLocaleString(undefined, { minimumFractionDigits: 2, maximumFractionDigits: 2 })
      : '--';

  return (
    <>
      <header className="border-b border-deep-border bg-deep-card">
        <div className="max-w-6xl mx-auto px-4 py-4 flex items-center justify-between">
          {/* Logo */}
          <div className="flex items-center gap-3">
            <div className="w-8 h-8 bg-deep-blue rounded-lg flex items-center justify-center">
              <span className="font-bold text-white">D</span>
            </div>
            <div>
              <h1 className="font-bold text-lg">DeepBook Sandbox</h1>
              <p className="text-xs text-gray-500">
                {session?.checkpoint
                  ? `Checkpoint ${(session.checkpoint / 1_000_000).toFixed(1)}M`
                  : 'Sandbox'}
              </p>
            </div>
          </div>

          {/* Wallet Balances */}
          <div className="flex items-center gap-4">
            <div className="flex items-center gap-4 px-4 py-2 bg-deep-dark rounded-lg border border-deep-border">
              <div className="text-right">
                <p className="text-xs text-gray-500">SUI</p>
                <p className="font-mono font-medium text-sm">
                  {fmt(balances?.sui_human)}
                </p>
              </div>
              <div className="w-px h-8 bg-deep-border" />
              <div className="text-right">
                <p className="text-xs text-gray-500">USDC</p>
                <p className="font-mono font-medium text-sm">
                  {fmt(balances?.usdc_human)}
                </p>
              </div>
              <div className="w-px h-8 bg-deep-border" />
              <div className="text-right">
                <p className="text-xs text-gray-500">DEEP</p>
                <p className="font-mono font-medium text-sm">
                  {fmt(balances?.deep_human)}
                </p>
              </div>
              <div className="w-px h-8 bg-deep-border" />
              <div className="text-right">
                <p className="text-xs text-gray-500">WAL</p>
                <p className="font-mono font-medium text-sm">
                  {fmt(balances?.wal_human)}
                </p>
              </div>
            </div>

            {/* Faucet Button */}
            <button
              onClick={() => setShowFaucet(true)}
              className="px-4 py-2 text-sm font-medium text-deep-blue border border-deep-blue/30 rounded-lg hover:bg-deep-blue/10 transition-colors flex items-center gap-2"
            >
              <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M12 6v6m0 0v6m0-6h6m-6 0H6" />
              </svg>
              Get Tokens
            </button>
          </div>
        </div>
      </header>

      {/* Faucet Modal */}
      <FaucetModal isOpen={showFaucet} onClose={() => setShowFaucet(false)} />
    </>
  );
}
