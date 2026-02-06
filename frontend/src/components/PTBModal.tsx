'use client';

import { useState } from 'react';
import type { PtbExecution, SwapMeta } from '@/types';
import { formatGas } from '@/lib/utils';
import {
  formatTypeArg,
  formatPackage,
  formatEventType,
  formatEventKey,
  formatEventValue,
  isRouterCommand,
} from '@/lib/ptb-utils';

interface PTBModalProps {
  isOpen: boolean;
  onClose: () => void;
  title: string;
  ptb?: PtbExecution;
  executionTime?: number;
  gasUsed?: string;
  swapMeta?: SwapMeta;
}

export function PTBModal({
  isOpen,
  onClose,
  title,
  ptb,
  executionTime,
  gasUsed,
  swapMeta,
}: PTBModalProps) {
  const [expandedEvents, setExpandedEvents] = useState<Set<number>>(new Set());

  if (!isOpen) return null;

  const isTwoHop = swapMeta?.route_type === 'two_hop';

  const toggleEvent = (index: number) => {
    setExpandedEvents((prev) => {
      const next = new Set(prev);
      if (next.has(index)) next.delete(index);
      else next.add(index);
      return next;
    });
  };

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center">
      {/* Backdrop */}
      <div
        className="absolute inset-0 bg-black/60 backdrop-blur-sm"
        onClick={onClose}
      />

      {/* Modal */}
      <div className="relative bg-deep-card border border-deep-border rounded-xl p-6 w-full max-w-3xl max-h-[85vh] overflow-auto">
        <div className="flex items-center justify-between mb-6">
          <h2 className="text-xl font-semibold">Transaction Explorer</h2>
          <button
            onClick={onClose}
            className="text-gray-400 hover:text-white transition-colors"
          >
            <svg className="w-6 h-6" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
            </svg>
          </button>
        </div>

        {/* A. Transaction Overview — Hero Section */}
        {swapMeta && (
          <div className="mb-6 p-4 rounded-xl bg-gradient-to-r from-blue-500/5 via-deep-card to-purple-500/5 border border-deep-border">
            <div className="flex items-center justify-between">
              {/* Input side */}
              <div className="text-center">
                <p className="text-xs text-gray-500 mb-1">Input</p>
                <p className="text-2xl font-mono font-bold">
                  {swapMeta.input_amount_human.toLocaleString(undefined, { maximumFractionDigits: 4 })}
                </p>
                <p className="text-sm text-gray-400 mt-0.5">{swapMeta.input_token.toUpperCase()}</p>
              </div>

              {/* Center arrow + route badge */}
              <div className="flex flex-col items-center gap-2 px-4">
                <div className="flex items-center gap-2">
                  <div className="w-8 h-px bg-gray-600" />
                  <span
                    className={`text-xs font-medium px-2 py-0.5 rounded-full ${
                      isTwoHop
                        ? 'bg-purple-500/20 text-purple-400 border border-purple-500/30'
                        : 'bg-blue-500/20 text-blue-400 border border-blue-500/30'
                    }`}
                  >
                    {isTwoHop ? 'Router' : 'Direct'}
                  </span>
                  <div className="w-8 h-px bg-gray-600" />
                </div>
                <svg className="w-5 h-5 text-gray-500" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                  <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M14 5l7 7m0 0l-7 7m7-7H3" />
                </svg>
              </div>

              {/* Output side */}
              <div className="text-center">
                <p className="text-xs text-gray-500 mb-1">Output</p>
                <p className="text-2xl font-mono font-bold text-green-400">
                  {swapMeta.output_amount_human.toLocaleString(undefined, { maximumFractionDigits: 4 })}
                </p>
                <p className="text-sm text-gray-400 mt-0.5">{swapMeta.output_token.toUpperCase()}</p>
              </div>
            </div>

            {/* Route flow visualization for two-hop */}
            {isTwoHop && swapMeta.intermediate_amount != null && (
              <div className="mt-4 flex items-center justify-center gap-2 text-sm">
                <span className="px-2 py-1 bg-deep-dark rounded-md font-medium">
                  {swapMeta.input_token.toUpperCase()}
                </span>
                <span className="text-gray-500">&rarr;</span>
                <span className="px-2 py-1 bg-green-500/10 text-green-400 rounded-md font-mono">
                  {swapMeta.intermediate_amount.toFixed(2)} USDC
                </span>
                <span className="text-gray-500">&rarr;</span>
                <span className="px-2 py-1 bg-deep-dark rounded-md font-medium">
                  {swapMeta.output_token.toUpperCase()}
                </span>
              </div>
            )}
          </div>
        )}

        {/* Fallback title if no swapMeta */}
        {!swapMeta && (
          <div className="mb-6 p-3 bg-deep-dark rounded-lg">
            <p className="text-sm text-gray-400">Action</p>
            <p className="font-medium">{title}</p>
          </div>
        )}

        {ptb ? (
          <>
            {/* B. Stats Grid */}
            <div className={`grid gap-4 mb-6 ${swapMeta ? 'grid-cols-3' : 'grid-cols-2'}`}>
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
              {swapMeta && (
                <div className="p-3 bg-deep-dark rounded-lg">
                  <p className="text-xs text-gray-500">Price Impact</p>
                  <p className={`font-mono font-medium ${swapMeta.price_impact_bps > 50 ? 'text-yellow-400' : 'text-green-400'}`}>
                    {swapMeta.price_impact_bps.toFixed(1)} bps
                  </p>
                </div>
              )}
            </div>

            {/* C. PTB Commands */}
            <div className="mb-6">
              <h3 className="text-sm text-gray-400 mb-3">PTB Commands</h3>
              <div className="space-y-2">
                {ptb.commands.map((cmd, i) => {
                  const isRouter = cmd.package ? isRouterCommand(cmd.package) : false;
                  return (
                    <div
                      key={i}
                      className={`p-3 bg-deep-dark rounded-lg border-l-2 ${
                        isRouter ? 'border-purple-500' : 'border-deep-blue'
                      }`}
                    >
                      <div className="flex items-center gap-2 mb-1">
                        <span className="text-xs bg-deep-blue/20 text-deep-blue px-2 py-0.5 rounded">
                          {cmd.index}
                        </span>
                        <span className="font-mono text-sm font-medium">
                          {cmd.command_type}
                        </span>
                        {isRouter && (
                          <span className="text-xs bg-purple-500/20 text-purple-400 px-2 py-0.5 rounded-full border border-purple-500/30">
                            Router Contract
                          </span>
                        )}
                      </div>
                      {(cmd.package || cmd.module || cmd.function) && (
                        <p className="text-xs text-gray-500 font-mono mb-1">
                          <span className="text-gray-400">{cmd.package ? formatPackage(cmd.package) : ''}</span>
                          {cmd.module && <span className="text-gray-500">::{cmd.module}</span>}
                          {cmd.function && <span className="text-gray-400">::{cmd.function}</span>}
                        </p>
                      )}
                      {cmd.type_args && cmd.type_args.length > 0 && (
                        <div className="flex flex-wrap gap-1 mt-1">
                          {cmd.type_args.map((arg, j) => (
                            <span
                              key={j}
                              className="text-xs bg-deep-card px-1.5 py-0.5 rounded font-mono text-gray-300"
                            >
                              {formatTypeArg(arg)}
                            </span>
                          ))}
                        </div>
                      )}
                      {cmd.description && (
                        <p className="text-sm text-gray-400 mt-1.5">
                          {cmd.description}
                        </p>
                      )}
                    </div>
                  );
                })}
              </div>
            </div>

            {/* D. Events (expandable) */}
            {ptb.events && ptb.events.length > 0 && (
              <div className="mb-6">
                <h3 className="text-sm text-gray-400 mb-3">Events ({ptb.events.length})</h3>
                <div className="space-y-2">
                  {ptb.events.map((evt, i) => {
                    const isExpanded = expandedEvents.has(i);
                    const eventName = formatEventType(evt.event_type);
                    const d = evt.data as Record<string, unknown>;
                    const leg = d?.leg as string | undefined;
                    const isOrderFilled = eventName === 'OrderFilled';

                    // Human-readable fields from enriched backend data
                    const direction = d?.direction as string | undefined;
                    const baseHuman = d?.base_quantity_human as string | undefined;
                    const quoteHuman = d?.quote_quantity_human as string | undefined;
                    const baseToken = d?.base_token as string | undefined;

                    // Fields to hide in expanded view (shown in summary instead)
                    const hiddenKeys = new Set([
                      'base_quantity', 'quote_quantity',
                      'base_quantity_human', 'quote_quantity_human',
                      'taker_is_bid', 'leg',
                    ]);

                    return (
                      <div key={i} className="bg-deep-dark rounded-lg overflow-hidden">
                        <button
                          onClick={() => toggleEvent(i)}
                          className="w-full p-3 flex items-center justify-between text-left hover:bg-deep-dark/80 transition-colors"
                        >
                          <div className="flex items-center gap-2 flex-wrap">
                            <span className="text-sm font-mono text-deep-blue">{eventName}</span>
                            {leg === 'first' && (
                              <span className="text-xs bg-blue-500/20 text-blue-400 px-1.5 py-0.5 rounded-full">
                                Leg 1
                              </span>
                            )}
                            {leg === 'second' && (
                              <span className="text-xs bg-purple-500/20 text-purple-400 px-1.5 py-0.5 rounded-full">
                                Leg 2
                              </span>
                            )}
                            {isOrderFilled && d?.pool_id != null && (
                              <span className="text-xs text-gray-500">
                                {String(d.pool_id)}
                              </span>
                            )}
                            {isOrderFilled && direction && (
                              <span className="text-xs text-gray-400">
                                &mdash; {direction}
                              </span>
                            )}
                          </div>
                          <svg
                            className={`w-4 h-4 text-gray-500 transition-transform flex-shrink-0 ${isExpanded ? 'rotate-180' : ''}`}
                            fill="none"
                            stroke="currentColor"
                            viewBox="0 0 24 24"
                          >
                            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M19 9l-7 7-7-7" />
                          </svg>
                        </button>

                        {isExpanded && evt.data && (
                          <div className="px-3 pb-3 border-t border-deep-border">
                            {/* Human-readable amounts summary for OrderFilled */}
                            {isOrderFilled && baseHuman && quoteHuman && (
                              <div className="mt-2 mb-2 p-2 bg-deep-card rounded-md flex items-center justify-between text-sm">
                                <span className="font-mono font-medium">
                                  {baseHuman} {baseToken ?? ''}
                                </span>
                                <span className="text-gray-500 mx-2">&harr;</span>
                                <span className="font-mono font-medium text-green-400">
                                  {quoteHuman} USDC
                                </span>
                              </div>
                            )}
                            {/* Remaining fields */}
                            <div className="mt-2 space-y-1">
                              {Object.entries(evt.data)
                                .filter(([key]) => !hiddenKeys.has(key))
                                .map(([key, value]) => (
                                  <div key={key} className="flex justify-between text-xs">
                                    <span className="text-gray-500">{formatEventKey(key)}</span>
                                    <span className="font-mono text-gray-300">
                                      {formatEventValue(key, value)}
                                    </span>
                                  </div>
                                ))}
                            </div>
                          </div>
                        )}
                      </div>
                    );
                  })}
                </div>
              </div>
            )}

            {/* E. Summary */}
            <div
              className={`p-3 rounded-lg border ${
                isTwoHop
                  ? 'bg-purple-500/10 border-purple-500/30'
                  : 'bg-green-500/10 border-green-500/30'
              }`}
            >
              <div className="flex items-center gap-2">
                <svg className={`w-5 h-5 ${isTwoHop ? 'text-purple-400' : 'text-green-500'}`} fill="none" stroke="currentColor" viewBox="0 0 24 24">
                  <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M5 13l4 4L19 7" />
                </svg>
                <span className={`font-medium ${isTwoHop ? 'text-purple-400' : 'text-green-400'}`}>
                  {ptb.status || 'Success'}
                </span>
                {isTwoHop && (
                  <span className="text-xs bg-purple-500/20 text-purple-400 px-2 py-0.5 rounded-full border border-purple-500/30">
                    Router Contract
                  </span>
                )}
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
            Simulated execution in sandbox environment &bull; No on-chain state modified
          </p>
        </div>
      </div>
    </div>
  );
}
