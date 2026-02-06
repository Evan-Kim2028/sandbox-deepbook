'use client';

import { Header } from '@/components/Header';
import { DebugPoolPanel } from '@/components/DebugPoolPanel';
import { SwapCard } from '@/components/SwapCard';
import { OrderBook } from '@/components/OrderBook';
import { ActivityFeed } from '@/components/ActivityFeed';
import { useSession } from '@/hooks/useSession';

export default function Home() {
  const { session, isLoading } = useSession();

  return (
    <main className="min-h-screen bg-deep-dark">
      <Header />

      <div className="max-w-6xl mx-auto px-4 py-8">
        {/* Sandbox Banner */}
        <div className="mb-6 p-3 bg-deep-blue/10 border border-deep-blue/30 rounded-lg text-center">
          <span className="text-deep-blue font-medium">
            Sandbox Mode
          </span>
          <span className="text-gray-400 ml-2">
            Forked from mainnet &bull; Move VM execution &bull; Smart order routing
          </span>
        </div>

        <DebugPoolPanel />

        {/* Main Content */}
        <div className="grid grid-cols-1 lg:grid-cols-3 gap-6">
          {/* Left: Order Book */}
          <div className="lg:col-span-2">
            <OrderBook />
          </div>

          {/* Right: Swap Card */}
          <div>
            <SwapCard sessionId={session?.session_id} />
          </div>
        </div>

        {/* Activity Feed */}
        <div className="mt-6">
          <ActivityFeed />
        </div>
      </div>
    </main>
  );
}
