module router::debug_token;

/// Fixed debug token type used for local VM pool creation and routing.
public struct DEBUG_TOKEN has key {
    id: sui::object::UID,
}

/// Initialize a shared DEBUG treasury for local-VM minting.
///
/// This is idempotent at the backend level (called once per VM instance).
public fun init_for_router(
    registry: &mut sui::coin_registry::CoinRegistry,
    decimals: u8,
    symbol_bytes: vector<u8>,
    name_bytes: vector<u8>,
    description_bytes: vector<u8>,
    icon_url_bytes: vector<u8>,
    ctx: &mut sui::tx_context::TxContext,
): sui::coin::TreasuryCap<DEBUG_TOKEN> {
    let (builder, cap) = sui::coin_registry::new_currency<DEBUG_TOKEN>(
        registry,
        decimals,
        std::string::utf8(symbol_bytes),
        std::string::utf8(name_bytes),
        std::string::utf8(description_bytes),
        std::string::utf8(icon_url_bytes),
        ctx
    );
    sui::coin_registry::finalize_and_delete_metadata_cap<DEBUG_TOKEN>(builder, ctx);
    cap
}
