/** Utility functions for formatting PTB (Programmable Transaction Block) data */

const KNOWN_PACKAGES: Record<string, string> = {
  '0x1': 'Move Stdlib',
  '0x2': 'Sui Framework',
  '0x3': 'Sui System',
  router: 'Router Contract',
  '0x2c8d603bc51326b8c13cef9dd07031a408a48dddb541963357661df5d3204809': 'DeepBook V3',
};

const KNOWN_TOKENS: Record<string, string> = {
  '0x2::sui::SUI': 'SUI',
  '0xdba34672e30cb065b1f93e3ab55318768fd6fef66c15942c9f7cb846e2f900e7::usdc::USDC': 'USDC',
  '0x356a26eb9e012a68958082340d4c4116e7f55615cf27affcff209cf0ae544f59::wal::WAL': 'WAL',
  '0xdeeb7a4662eec9f2f3def03fb937a663dddaa2e215b8078a284d026b7946c270::deep::DEEP': 'DEEP',
};

/**
 * Extract human-readable token name from a full type arg.
 * "0xdba34...::usdc::USDC" → "USDC"
 */
export function formatTypeArg(typeArg: string | undefined): string {
  if (!typeArg) return '?';
  if (KNOWN_TOKENS[typeArg]) return KNOWN_TOKENS[typeArg];
  // Fallback: grab the last segment after `::`
  const parts = typeArg.split('::');
  return parts.length >= 1 ? parts[parts.length - 1] : typeArg;
}

/**
 * Format a package address into a human-readable label.
 * "0x2" → "Sui Framework", "router" → "Router Contract"
 */
export function formatPackage(pkg: string): string {
  if (KNOWN_PACKAGES[pkg]) return KNOWN_PACKAGES[pkg];
  if (pkg.length > 10) return `${pkg.slice(0, 8)}...`;
  return pkg;
}

/**
 * Extract the event name from a full event type string.
 * "0x2c8d...::pool::OrderFilled" → "OrderFilled"
 */
export function formatEventType(eventType: string | undefined): string {
  if (!eventType) return 'Unknown';
  const parts = eventType.split('::');
  return parts.length >= 1 ? parts[parts.length - 1] : eventType;
}

/**
 * Format an event data value for display.
 */
export function formatEventValue(key: string, value: unknown): string {
  if (typeof value === 'boolean') return value ? 'Yes' : 'No';
  if (typeof value === 'number') {
    if (key.includes('quantity') || key.includes('amount')) {
      return value.toLocaleString();
    }
    return String(value);
  }
  if (typeof value === 'string') {
    if (value.startsWith('0x') && value.length > 20) {
      return `${value.slice(0, 10)}...${value.slice(-6)}`;
    }
    return value;
  }
  return JSON.stringify(value);
}

/**
 * Prettify an event data key for display.
 * "taker_is_bid" → "Taker Is Bid"
 */
export function formatEventKey(key: string): string {
  return key
    .split('_')
    .map((w) => w.charAt(0).toUpperCase() + w.slice(1))
    .join(' ');
}

/**
 * Check if a command is from the router contract.
 */
export function isRouterCommand(pkg: string): boolean {
  return pkg === 'router';
}
