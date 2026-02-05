'use client';

import type { PtbExecution } from '@/types';
import { formatGas } from '@/lib/utils';

interface PTBModalProps {
  isOpen: boolean;
  onClose: () => void;
  title: string;
  ptb?: PtbExecution;
  executionTime?: number;
  gasUsed?: string;
}

export function PTBModal({
  isOpen,
  onClose,
  title,
  ptb,
  executionTime,
  gasUsed,
}: PTBModalProps) {
  if (!isOpen) return null;

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center">
      {/* Backdrop */}
      <div
        className="absolute inset-0 bg-black/60 backdrop-blur-sm"
        onClick={onClose}
      />

      {/* Modal */}
      <div className="relative bg-deep-card border border-deep-border rounded-xl p-6 w-full max-w-lg max-h-[80vh] overflow-auto">
        <div className="flex items-center justify-between mb-6">
          <h2 className="text-xl font-semibold">Transaction Details</h2>
          <button
            onClick={onClose}
            className="text-gray-400 hover:text-white transition-colors"
          >
            <svg className="w-6 h-6" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
            </svg>
          </button>
        </div>

        {/* Title */}
        <div className="mb-6 p-3 bg-deep-dark rounded-lg">
          <p className="text-sm text-gray-400">Action</p>
          <p className="font-medium">{title}</p>
        </div>

        {ptb ? (
          <>
            {/* Commands */}
            <div className="mb-6">
              <h3 className="text-sm text-gray-400 mb-3">PTB Commands</h3>
              <div className="space-y-2">
                {ptb.commands.map((cmd, i) => (
                  <div
                    key={i}
                    className="p-3 bg-deep-dark rounded-lg border-l-2 border-deep-blue"
                  >
                    <div className="flex items-center gap-2 mb-1">
                      <span className="text-xs bg-deep-blue/20 text-deep-blue px-2 py-0.5 rounded">
                        {cmd.index}
                      </span>
                      <span className="font-mono text-sm font-medium">
                        {cmd.command_type}
                      </span>
                    </div>
                    {(cmd.package || cmd.module || cmd.function) && (
                      <p className="text-xs text-gray-500 font-mono mb-1">
                        {cmd.package && `${cmd.package.slice(0, 8)}...`}
                        {cmd.module && `::${cmd.module}`}
                        {cmd.function && `::${cmd.function}`}
                      </p>
                    )}
                    <p className="text-sm text-gray-400 font-mono">
                      {cmd.description}
                    </p>
                  </div>
                ))}
              </div>
            </div>

            {/* Events */}
            {ptb.events && ptb.events.length > 0 && (
              <div className="mb-6">
                <h3 className="text-sm text-gray-400 mb-3">Events ({ptb.events.length})</h3>
                <div className="space-y-2">
                  {ptb.events.map((evt, i) => (
                    <div key={i} className="p-2 bg-deep-dark rounded-lg text-xs font-mono">
                      <span className="text-deep-blue">{evt.type}</span>
                    </div>
                  ))}
                </div>
              </div>
            )}

            {/* Execution Stats */}
            <div className="grid grid-cols-2 gap-4 mb-6">
              {executionTime != null && (
                <div className="p-3 bg-deep-dark rounded-lg">
                  <p className="text-xs text-gray-500">Execution Time</p>
                  <p className="font-mono font-medium text-green-400">
                    {executionTime}ms
                  </p>
                </div>
              )}
              {gasUsed && (
                <div className="p-3 bg-deep-dark rounded-lg">
                  <p className="text-xs text-gray-500">Gas Used</p>
                  <p className="font-mono font-medium">
                    {formatGas(gasUsed)}
                  </p>
                </div>
              )}
            </div>

            {/* Summary */}
            <div className="p-3 bg-green-500/10 border border-green-500/30 rounded-lg">
              <div className="flex items-center gap-2">
                <svg className="w-5 h-5 text-green-500" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                  <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M5 13l4 4L19 7" />
                </svg>
                <span className="font-medium text-green-400">{ptb.status || 'Success'}</span>
              </div>
              {ptb.summary && (
                <p className="text-sm text-gray-400 mt-1">{ptb.summary}</p>
              )}
            </div>
          </>
        ) : (
          <div className="py-8 text-center text-gray-500">
            No PTB details available for this transaction.
          </div>
        )}

        {/* Footer */}
        <div className="mt-6 pt-4 border-t border-deep-border">
          <p className="text-xs text-gray-500 text-center">
            Simulated execution in sandbox environment • No on-chain state modified
          </p>
        </div>
      </div>
    </div>
  );
}
