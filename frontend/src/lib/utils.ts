import { clsx, type ClassValue } from 'clsx';
import { twMerge } from 'tailwind-merge';

export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs));
}

export function formatBalance(amount: string, decimals: number): string {
  const value = parseFloat(amount) / Math.pow(10, decimals);
  if (value === 0) return '0';
  if (value < 0.01) return '<0.01';
  return value.toLocaleString(undefined, {
    minimumFractionDigits: 2,
    maximumFractionDigits: 2,
  });
}

export function formatAddress(address: string): string {
  if (!address) return '';
  return `${address.slice(0, 6)}...${address.slice(-4)}`;
}

export function formatGas(mist: string): string {
  const value = parseInt(mist);
  if (value < 1000) return `${value} MIST`;
  if (value < 1000000) return `${(value / 1000).toFixed(2)}K MIST`;
  return `${(value / 1000000).toFixed(2)}M MIST`;
}

export function formatDuration(ms: number): string {
  if (ms < 1000) return `${ms}ms`;
  return `${(ms / 1000).toFixed(2)}s`;
}
