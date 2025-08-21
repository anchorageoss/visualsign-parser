#![allow(dead_code)]

crate::chain_config! {
  config CETUS_CONFIG as Config;

  cetus_mainnet => {
      package_id => 0xb2db7142fa83210a7d78d9c12ac49c043b3cbbd482224fea6e3da00aa5a5ae2d,
      modules: {
        pool_script_v2 => PoolScriptV2Functions: {
          swap_a2b as SwapA2B => SwapA2BIndexes(
                amount_out as AmountOut: u64 => 5 => get_amount_out,
                max_amount_in as MaxAmountIn: u64 => 6 => get_max_amount_in,
                sqrt_price_limit as SqrtPriceLimit: u128 => 7 => get_sqrt_price_limit,
          ),
          swap_b2a as SwapB2A => SwapB2AIndexes(
                amount_in as AmountIn: u64 => 5 => get_amount_in,
                min_amount_out as MinAmountOut: u64 => 6 => get_min_amount_out,
                sqrt_price_limit as SqrtPriceLimit: u128 => 7 => get_sqrt_price_limit,
          ),
        },
      }
  },
}
