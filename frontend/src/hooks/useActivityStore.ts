'use client';

import { create } from 'zustand';
import type { Activity, PTBDetails } from '@/types';

interface ActivityStore {
  activities: Activity[];
  addActivity: (activity: Omit<Activity, 'id' | 'timestamp'>) => void;
  clearActivities: () => void;
}

// Simple ID generator
let activityId = 0;

export const useActivityStore = create<ActivityStore>((set) => ({
  activities: [],

  addActivity: (activity) =>
    set((state) => ({
      activities: [
        {
          ...activity,
          id: String(++activityId),
          timestamp: new Date(),
        },
        ...state.activities,
      ].slice(0, 50), // Keep last 50 activities
    })),

  clearActivities: () => set({ activities: [] }),
}));
