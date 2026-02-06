'use client';

import { useState } from 'react';
import { useActivityStore } from '@/hooks/useActivityStore';
import { PTBModal } from './PTBModal';
import type { Activity } from '@/types';

const PAGE_SIZE = 5;

export function ActivityFeed() {
  const activities = useActivityStore((s) => s.activities);
  const [selectedActivity, setSelectedActivity] = useState<Activity | null>(null);
  const [page, setPage] = useState(0);

  const totalPages = Math.max(1, Math.ceil(activities.length / PAGE_SIZE));
  const pageActivities = activities.slice(page * PAGE_SIZE, (page + 1) * PAGE_SIZE);

  if (activities.length === 0) {
    return (
      <div className="bg-deep-card border border-deep-border rounded-xl p-4">
        <h2 className="text-lg font-semibold mb-4">Transaction History</h2>
        <p className="text-gray-500 text-center py-8">
          No transactions yet. Request tokens or make a swap to get started!
        </p>
      </div>
    );
  }

  return (
    <>
      <div className="bg-deep-card border border-deep-border rounded-xl p-4">
        <div className="flex items-center justify-between mb-4">
          <h2 className="text-lg font-semibold">Transaction History</h2>
          <span className="text-sm text-gray-500">{activities.length} transactions</span>
        </div>

        <div className="space-y-3">
          {pageActivities.map((activity) => {
            const isTwoHop = activity.swapMeta?.route_type === 'two_hop';

            return (
              <div
                key={activity.id}
                className="flex items-center justify-between p-3 bg-deep-dark rounded-lg hover:bg-deep-dark/80 transition-colors"
              >
                <div className="flex items-center gap-3">
                  {/* Icon with optional router dot badge */}
                  <div className="relative">
                    <div
                      className={`w-10 h-10 rounded-full flex items-center justify-center ${
                        activity.type === 'swap'
                          ? 'bg-blue-500/20'
                          : 'bg-green-500/20'
                      }`}
                    >
                      {activity.type === 'swap' ? (
                        <svg
                          className="w-5 h-5 text-blue-400"
                          fill="none"
                          stroke="currentColor"
                          viewBox="0 0 24 24"
                        >
                          <path
                            strokeLinecap="round"
                            strokeLinejoin="round"
                            strokeWidth={2}
                            d="M7 16V4m0 0L3 8m4-4l4 4m6 0v12m0 0l4-4m-4 4l-4-4"
                          />
                        </svg>
                      ) : (
                        <svg
                          className="w-5 h-5 text-green-400"
                          fill="none"
                          stroke="currentColor"
                          viewBox="0 0 24 24"
                        >
                          <path
                            strokeLinecap="round"
                            strokeLinejoin="round"
                            strokeWidth={2}
                            d="M12 8c-1.657 0-3 .895-3 2s1.343 2 3 2 3 .895 3 2-1.343 2-3 2m0-8c1.11 0 2.08.402 2.599 1M12 8V7m0 1v8m0 0v1m0-1c-1.11 0-2.08-.402-2.599-1M21 12a9 9 0 11-18 0 9 9 0 0118 0z"
                          />
                        </svg>
                      )}
                    </div>
                    {/* Purple dot badge for two-hop swaps */}
                    {isTwoHop && (
                      <div className="absolute -top-0.5 -right-0.5 w-3 h-3 bg-purple-500 rounded-full border-2 border-deep-dark" />
                    )}
                  </div>

                  {/* Details */}
                  <div>
                    <p className="font-medium flex items-center gap-2">
                      {activity.description}
                      {isTwoHop && (
                        <span className="text-xs bg-purple-500/20 text-purple-400 px-1.5 py-0.5 rounded-full border border-purple-500/30">
                          Router
                        </span>
                      )}
                    </p>
                    <p className="text-sm text-gray-500">
                      {activity.execution_time_ms != null && (
                        <>
                          <span className="text-green-400">{activity.execution_time_ms}ms</span>
                          {' \u2022 '}
                        </>
                      )}
                      {activity.timestamp.toLocaleTimeString()}
                    </p>
                  </div>
                </div>

                {/* View PTB Button */}
                {activity.ptb_execution && (
                  <button
                    onClick={() => setSelectedActivity(activity)}
                    className="px-3 py-1.5 text-sm text-deep-blue hover:bg-deep-blue/10 rounded-lg transition-colors flex items-center gap-1"
                  >
                    <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                      <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M15 12a3 3 0 11-6 0 3 3 0 016 0z" />
                      <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M2.458 12C3.732 7.943 7.523 5 12 5c4.478 0 8.268 2.943 9.542 7-1.274 4.057-5.064 7-9.542 7-4.477 0-8.268-2.943-9.542-7z" />
                    </svg>
                    View PTB
                  </button>
                )}
              </div>
            );
          })}
        </div>

        {/* Pagination */}
        {totalPages > 1 && (
          <div className="flex items-center justify-between mt-4 pt-3 border-t border-deep-border">
            <button
              onClick={() => setPage((p) => Math.max(0, p - 1))}
              disabled={page === 0}
              className="px-3 py-1.5 text-sm rounded-lg border border-deep-border hover:bg-deep-dark disabled:opacity-30 disabled:cursor-not-allowed transition-colors"
            >
              Prev
            </button>
            <span className="text-sm text-gray-500">
              Page {page + 1} of {totalPages}
            </span>
            <button
              onClick={() => setPage((p) => Math.min(totalPages - 1, p + 1))}
              disabled={page >= totalPages - 1}
              className="px-3 py-1.5 text-sm rounded-lg border border-deep-border hover:bg-deep-dark disabled:opacity-30 disabled:cursor-not-allowed transition-colors"
            >
              Next
            </button>
          </div>
        )}
      </div>

      {/* PTB Modal */}
      {selectedActivity && (
        <PTBModal
          isOpen={!!selectedActivity}
          onClose={() => setSelectedActivity(null)}
          title={selectedActivity.description}
          ptb={selectedActivity.ptb_execution}
          executionTime={selectedActivity.execution_time_ms}
          gasUsed={selectedActivity.gas_used}
          swapMeta={selectedActivity.swapMeta}
        />
      )}
    </>
  );
}
