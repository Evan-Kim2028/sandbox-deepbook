'use client';

import { useState } from 'react';
import { useActivityStore } from '@/hooks/useActivityStore';
import { PTBModal } from './PTBModal';
import type { Activity } from '@/types';

export function ActivityFeed() {
  const activities = useActivityStore((s) => s.activities);
  const [selectedActivity, setSelectedActivity] = useState<Activity | null>(null);

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
        <h2 className="text-lg font-semibold mb-4">Transaction History</h2>

        <div className="space-y-3">
          {activities.map((activity) => (
            <div
              key={activity.id}
              className="flex items-center justify-between p-3 bg-deep-dark rounded-lg hover:bg-deep-dark/80 transition-colors"
            >
              <div className="flex items-center gap-3">
                {/* Icon */}
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

                {/* Details */}
                <div>
                  <p className="font-medium">{activity.description}</p>
                  <p className="text-sm text-gray-500">
                    <span className="text-green-400">{activity.execution_time_ms}ms</span>
                    {' • '}
                    {activity.timestamp.toLocaleTimeString()}
                  </p>
                </div>
              </div>

              {/* View PTB Button */}
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
            </div>
          ))}
        </div>
      </div>

      {/* PTB Modal */}
      {selectedActivity && (
        <PTBModal
          isOpen={!!selectedActivity}
          onClose={() => setSelectedActivity(null)}
          title={selectedActivity.description}
          ptb={selectedActivity.ptb_details}
          executionTime={selectedActivity.execution_time_ms}
          gasUsed={selectedActivity.gas_used}
        />
      )}
    </>
  );
}
