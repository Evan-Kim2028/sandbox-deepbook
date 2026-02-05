'use client';

import { useState } from 'react';
import { useBalances } from '@/hooks/useSession';
import { formatBalance } from '@/lib/utils';
import { FaucetModal } from './FaucetModal';

export function Header() {
  const { balances } = useBalances();
  const [showFaucet, setShowFaucet] = useState(false);

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
              <p className="text-xs text-gray-500">SUI/USDC Pool</p>
            </div>
          </div>

          {/* Wallet Balances */}
          <div className="flex items-center gap-4">
            <div className="flex items-center gap-6 px-4 py-2 bg-deep-dark rounded-lg border border-deep-border">
              <div className="text-right">
                <p className="text-xs text-gray-500">SUI</p>
                <p className="font-mono font-medium">
                  {formatBalance(balances?.sui || '0', 9)}
                </p>
              </div>
              <div className="w-px h-8 bg-deep-border" />
              <div className="text-right">
                <p className="text-xs text-gray-500">USDC</p>
                <p className="font-mono font-medium">
                  {formatBalance(balances?.usdc || '0', 6)}
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
