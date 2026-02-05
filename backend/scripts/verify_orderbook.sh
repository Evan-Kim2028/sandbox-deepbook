#!/bin/bash
# Verify DeepBook orderbook state by calling view functions via dev-inspect
# This queries CURRENT on-chain state (not historical)

set -e

DEEPBOOK_PACKAGE="0x2c8d603bc51326b8c13cef9dd07031a408a48dddb541963357661df5d3204809"
SUI_USDC_POOL="0xe05dafb5133bcffb8d59f4e12465dc0e9faeaa05e3e342a08fe135800e3e4407"
CLOCK="0x6"

# Type arguments for SUI/USDC pool
BASE_TYPE="0x2::sui::SUI"
QUOTE_TYPE="0xdba34672e30cb065b1f93e3ab55318768fd6fef66c15942c9f7cb846e2f900e7::usdc::USDC"

echo "=== DeepBook Orderbook Verification ==="
echo "Pool: SUI/USDC"
echo "Package: $DEEPBOOK_PACKAGE"
echo ""

# 1. Get pool book params (tick_size, lot_size, min_size)
echo "1. Pool Book Params (tick_size, lot_size, min_size):"
sui client ptb \
  --assign pool_id "@$SUI_USDC_POOL" \
  --move-call "${DEEPBOOK_PACKAGE}::pool::pool_book_params<${BASE_TYPE}, ${QUOTE_TYPE}>" pool_id \
  --dev-inspect 2>&1 | grep -A 20 "Return values"

echo ""

# 2. Get mid price
echo "2. Mid Price:"
sui client ptb \
  --assign pool_id "@$SUI_USDC_POOL" \
  --assign clock_id "@$CLOCK" \
  --move-call "${DEEPBOOK_PACKAGE}::pool::mid_price<${BASE_TYPE}, ${QUOTE_TYPE}>" pool_id clock_id \
  --dev-inspect 2>&1 | grep -A 10 "Return values"

echo ""

# 3. Get level2 orderbook (10 ticks from mid)
echo "3. Level2 Orderbook (10 ticks from mid):"
echo "   Returns: (bid_prices, bid_quantities, ask_prices, ask_quantities)"
sui client ptb \
  --assign pool_id "@$SUI_USDC_POOL" \
  --assign clock_id "@$CLOCK" \
  --move-call "${DEEPBOOK_PACKAGE}::pool::get_level2_ticks_from_mid<${BASE_TYPE}, ${QUOTE_TYPE}>" pool_id 10u64 clock_id \
  --dev-inspect 2>&1 | grep -A 50 "Return values"

echo ""
echo "=== Verification Complete ==="
echo ""
echo "Note: This queries CURRENT on-chain state."
echo "To verify historical state (Feb 2, 2026 checkpoint 241056077),"
echo "compare against our cached JSONL files."
