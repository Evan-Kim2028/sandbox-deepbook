'use client';

import { useState } from 'react';
import { toast } from 'sonner';
import { useDebugPoolStatus, useFaucet } from '@/hooks/useSession';
import { useActivityStore } from '@/hooks/useActivityStore';

interface FaucetModalProps {
  isOpen: boolean;
  onClose: () => void;
}

type FaucetToken = {
  id: string;
  name: string;
  amount: string;
  decimals: number;
  icon: string;
  color: string;
};

export function FaucetModal({ isOpen, onClose }: FaucetModalProps) {
  const { debugPool } = useDebugPoolStatus();
  const debugDecimals = debugPool?.token_decimals ?? 9;
  const tokens: FaucetToken[] = [
    { id: 'sui', name: 'SUI', amount: '10', decimals: 9, icon: '◎', color: 'text-blue-400' },
    { id: 'usdc', name: 'USDC', amount: '100', decimals: 6, icon: '$', color: 'text-green-400' },
    { id: 'deep', name: 'DEEP', amount: '100', decimals: 6, icon: 'D', color: 'text-cyan-400' },
    { id: 'wal', name: 'WAL', amount: '50', decimals: 9, icon: 'W', color: 'text-purple-400' },
    {
      id: (debugPool?.token_symbol ?? 'DBG').toLowerCase(),
      name: debugPool?.token_symbol ?? 'DBG',
      amount: '100',
      decimals: debugDecimals,
      icon: '◇',
      color: 'text-amber-400',
    },
  ];
  const [selectedToken, setSelectedToken] = useState<string>(tokens[0].id);
  const faucet = useFaucet();
  const addActivity = useActivityStore((s) => s.addActivity);

  if (!isOpen) return null;

  const handleRequest = async () => {
    const token = tokens.find((t) => t.id === selectedToken);
    if (!token) return;
    const amountRaw = Math.round(parseFloat(token.amount) * Math.pow(10, token.decimals)).toString();

    try {
      const result = await faucet.mutateAsync({
        token: token.name.toLowerCase(),
        amount: amountRaw,
      });

      addActivity({
        type: 'faucet',
        description: `Received ${token.amount} ${token.name} from faucet`,
      });

      const balanceDisplay = result.new_balance_human != null
        ? `New balance: ${result.new_balance_human}`
        : '';

      toast.success(
        <div>
          <p className="font-medium">Tokens Received!</p>
          <p className="text-sm text-gray-400">
            +{token.amount} {token.name}
          </p>
          {balanceDisplay && (
            <p className="text-xs text-gray-500 mt-1">{balanceDisplay}</p>
          )}
        </div>
      );

      onClose();
    } catch (error) {
      toast.error('Failed to request tokens');
    }
  };

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center">
      {/* Backdrop */}
      <div
        className="absolute inset-0 bg-black/60 backdrop-blur-sm"
        onClick={onClose}
      />

      {/* Modal */}
      <div className="relative bg-deep-card border border-deep-border rounded-xl p-6 w-full max-w-md">
        <div className="flex items-center justify-between mb-6">
          <h2 className="text-xl font-semibold">Request Test Tokens</h2>
          <button
            onClick={onClose}
            className="text-gray-400 hover:text-white transition-colors"
          >
            <svg className="w-6 h-6" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
            </svg>
          </button>
        </div>

        <p className="text-gray-400 text-sm mb-6">
          Select a token to receive. Faucet coin objects are created via local MoveVM PTB execution.
        </p>

        {/* Token Selection */}
        <div className="space-y-3 mb-6">
          {tokens.map((token) => (
            <button
              key={token.id}
              onClick={() => setSelectedToken(token.id)}
              className={`w-full p-4 rounded-lg border transition-all flex items-center justify-between ${
                selectedToken === token.id
                  ? 'border-deep-blue bg-deep-blue/10'
                  : 'border-deep-border hover:border-gray-600'
              }`}
            >
              <div className="flex items-center gap-3">
                <div className={`w-10 h-10 rounded-full bg-deep-dark flex items-center justify-center ${token.color}`}>
                  <span className="text-lg font-bold">{token.icon}</span>
                </div>
                <div className="text-left">
                  <p className="font-medium">{token.name}</p>
                  <p className="text-sm text-gray-500">+{token.amount} tokens</p>
                </div>
              </div>
              {selectedToken === token.id && (
                <svg className="w-5 h-5 text-deep-blue" fill="currentColor" viewBox="0 0 20 20">
                  <path fillRule="evenodd" d="M10 18a8 8 0 100-16 8 8 0 000 16zm3.707-9.293a1 1 0 00-1.414-1.414L9 10.586 7.707 9.293a1 1 0 00-1.414 1.414l2 2a1 1 0 001.414 0l4-4z" clipRule="evenodd" />
                </svg>
              )}
            </button>
          ))}
        </div>

        {/* Request Button */}
        <button
          onClick={handleRequest}
          disabled={faucet.isPending}
          className="w-full py-3 bg-deep-blue hover:bg-deep-blue/90 disabled:bg-gray-700 rounded-lg font-semibold transition-colors"
        >
          {faucet.isPending ? (
            <span className="flex items-center justify-center gap-2">
              <svg className="animate-spin h-5 w-5" viewBox="0 0 24 24">
                <circle className="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" strokeWidth="4" fill="none" />
                <path className="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4zm2 5.291A7.962 7.962 0 014 12H0c0 3.042 1.135 5.824 3 7.938l3-2.647z" />
              </svg>
              Requesting...
            </span>
          ) : (
            'Request Tokens'
          )}
        </button>

        <p className="text-center text-xs text-gray-500 mt-4">
          Sandbox faucet • Unlimited requests
        </p>
      </div>
    </div>
  );
}
