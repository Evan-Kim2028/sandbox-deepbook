# DeepBook Sandbox Frontend Design

## Design Philosophy

**Keep it simple.** The goal is to demonstrate:
1. Mainnet fork works (real order book data)
2. Swaps execute instantly (sandbox speed)
3. No tokens needed (simulated balances)

Not building a production DEX - building a demo that delivers "aha moments".

---

## Layout (Single Page)

```
┌─────────────────────────────────────────────────────────────────────────┐
│  DeepBook Sandbox                              [Wallet: 100 SUI | 1000 USDC] │
├─────────────────────────────────────────────────────────────────────────┤
│                                                                         │
│   ┌─────────────────────────────┐    ┌─────────────────────────────┐   │
│   │                             │    │         SWAP                │   │
│   │      PRICE CHART            │    │  ┌─────────────────────┐   │   │
│   │      (Simple line)          │    │  │  USDC          ▼    │   │   │
│   │                             │    │  │  100                │   │   │
│   │                             │    │  └─────────────────────┘   │   │
│   │                             │    │           ↓                │   │
│   └─────────────────────────────┘    │  ┌─────────────────────┐   │   │
│                                      │  │  SUI           ▼    │   │   │
│   ┌─────────────────────────────┐    │  │  ~25.5              │   │   │
│   │       ORDER BOOK            │    │  └─────────────────────┘   │   │
│   │  ───────────────────────    │    │                            │   │
│   │  Asks (red)                 │    │  Price: 3.92 USDC/SUI     │   │
│   │  3.95  |████████  500       │    │  Impact: 0.05%            │   │
│   │  3.94  |██████    350       │    │                            │   │
│   │  3.93  |████      200       │    │  ┌─────────────────────┐   │   │
│   │  ─── 3.92 (mid) ───         │    │  │      SWAP           │   │   │
│   │  3.91  |█████     280       │    │  └─────────────────────┘   │   │
│   │  3.90  |███████   420       │    │                            │   │
│   │  3.89  |██████████ 600      │    │  Executing in sandbox...  │   │
│   │  Bids (green)               │    │                            │   │
│   └─────────────────────────────┘    └─────────────────────────────┘   │
│                                                                         │
│   ┌─────────────────────────────────────────────────────────────────┐   │
│   │  Recent Activity                                                │   │
│   │  ✓ Swapped 100 USDC → 25.5 SUI (0.045s) [View PTB]             │   │
│   │  ✓ Swapped 50 USDC → 12.7 SUI (0.038s) [View PTB]              │   │
│   └─────────────────────────────────────────────────────────────────┘   │
│                                                                         │
│   ┌─────────────────────────────────────────────────────────────────┐   │
│   │  🧪 This is a SANDBOX - forked from mainnet, no real funds      │   │
│   └─────────────────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────────────────┘
```

---

## Components (4 Total)

### 1. Header + Wallet Display
- Logo/title
- Simulated wallet balances (SUI, USDC)
- "Get Test Tokens" faucet button

### 2. Price Chart (Optional/Simple)
- Simple line chart showing recent price
- Can use lightweight-charts or just skip for MVP
- **MVP**: Static price display is fine

### 3. Order Book
- Bids (green) and Asks (red)
- Price | Size bars
- Mid price highlighted
- **Real data from forked mainnet state**

### 4. Swap Card
- From token selector (USDC/SUI)
- To token selector
- Amount input
- Estimated output
- Price impact
- **Big "SWAP" button**
- Loading state → Success toast with PTB link

### 5. Activity Feed
- Recent swaps with execution time
- "View PTB" expands to show transaction details
- Shows sandbox speed (sub-100ms)

---

## Tech Stack

```
Next.js 14 (App Router)
├── TailwindCSS (styling)
├── shadcn/ui (components)
├── React Query (data fetching)
├── lightweight-charts (optional, for price chart)
└── sonner (toast notifications)
```

---

## API Integration

```typescript
// hooks/useSandbox.ts

// Create session on page load
const { data: session } = useQuery({
  queryKey: ['session'],
  queryFn: () => api.createSession()
});

// Fetch balances
const { data: balances } = useQuery({
  queryKey: ['balances', session?.id],
  queryFn: () => api.getBalances(session.id),
  refetchInterval: 5000
});

// Fetch order book
const { data: orderBook } = useQuery({
  queryKey: ['orderbook'],
  queryFn: () => api.getOrderBook('SUI_USDC'),
  refetchInterval: 10000  // or WebSocket
});

// Execute swap
const swapMutation = useMutation({
  mutationFn: (params) => api.executeSwap(session.id, params),
  onSuccess: (data) => {
    toast.success(`Swapped! ${data.execution_time_ms}ms`);
    queryClient.invalidateQueries(['balances']);
  }
});
```

---

## User Flow

1. **Land on page** → Session created automatically
2. **See balances** → 100 SUI, 1000 USDC (simulated)
3. **See order book** → Real mainnet data (forked)
4. **Enter swap amount** → See estimated output
5. **Click SWAP** →
   - Button shows spinner
   - Backend executes PTB in sandbox
   - Toast: "✓ Swapped 100 USDC → 25.5 SUI (45ms)"
   - Balances update
6. **Click "View PTB"** → Modal shows transaction breakdown

---

## Notifications

```
┌─────────────────────────────────────┐
│ ✓ Swap Executed                     │
│                                     │
│ 100 USDC → 25.51 SUI                │
│ Execution: 45ms                     │
│ Gas used: 1,234,567 MIST            │
│                                     │
│ [View PTB Details]                  │
└─────────────────────────────────────┘
```

---

## PTB Details Modal

When user clicks "View PTB":

```
┌─────────────────────────────────────────────────┐
│ Transaction Details                        [X]  │
├─────────────────────────────────────────────────┤
│                                                 │
│ Commands:                                       │
│ ┌─────────────────────────────────────────────┐ │
│ │ 0. SplitCoins                               │ │
│ │    Split 100 USDC from wallet              │ │
│ ├─────────────────────────────────────────────┤ │
│ │ 1. MoveCall                                 │ │
│ │    deepbook::pool::swap_exact_base_for_quote│ │
│ │    Pool: 0xe05d...4407                     │ │
│ ├─────────────────────────────────────────────┤ │
│ │ 2. TransferObjects                          │ │
│ │    Transfer SUI to sender                  │ │
│ └─────────────────────────────────────────────┘ │
│                                                 │
│ Gas: 1,234,567 MIST                            │
│ Status: Success (simulated)                    │
│                                                 │
└─────────────────────────────────────────────────┘
```

---

## File Structure

```
frontend/
├── src/
│   ├── app/
│   │   ├── page.tsx           # Main page
│   │   ├── layout.tsx         # Root layout
│   │   └── globals.css        # Tailwind
│   ├── components/
│   │   ├── Header.tsx         # Logo + wallet
│   │   ├── SwapCard.tsx       # Main swap UI
│   │   ├── OrderBook.tsx      # Bid/ask display
│   │   ├── PriceChart.tsx     # Simple chart (optional)
│   │   ├── ActivityFeed.tsx   # Recent transactions
│   │   ├── PTBModal.tsx       # Transaction details
│   │   └── ui/                # shadcn components
│   ├── hooks/
│   │   ├── useSandbox.ts      # API hooks
│   │   └── useSession.ts      # Session management
│   ├── lib/
│   │   ├── api.ts             # Backend client
│   │   └── utils.ts           # Formatters
│   └── types/
│       └── index.ts           # TypeScript types
├── package.json
├── tailwind.config.js
└── next.config.js
```

---

## MVP Scope (v0.1)

**Include:**
- [x] Swap card with token selector
- [x] Simulated wallet balances
- [x] Execute swap → success toast
- [x] Activity feed with execution times

**Defer:**
- [ ] Order book visualization (v0.2)
- [ ] Price chart (v0.2)
- [ ] PTB details modal (v0.2)
- [ ] Faucet button (v0.2)

**MVP Goal**: User can swap USDC→SUI and see it worked in <100ms
