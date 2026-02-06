module router::router;

use deepbook::pool::{Self, Pool};
use sui::clock::Clock;

/// Quote a two-hop swap: A -> Q (intermediate) -> B
/// Returns (final_output_amount, intermediate_quote_amount)
public fun quote_two_hop<A, Q, B>(
    pool_aq: &Pool<A, Q>,
    pool_bq: &Pool<B, Q>,
    input_amount: u64,
    clock: &Clock,
): (u64, u64) {
    // Leg 1: Sell A for Q
    let (quote_out, _, _) = pool::get_quote_quantity_out<A, Q>(pool_aq, input_amount, clock);
    // Leg 2: Buy B with Q
    let (base_out, _, _) = pool::get_base_quantity_out<B, Q>(pool_bq, quote_out, clock);
    (base_out, quote_out)
}
