#![allow(dead_code)]

crate::chain_config! {
  config MOMENTUM_CONFIG as Config;

  momentum_mainnet => {
      package_id => 0xcf60a40f45d46fc1e828871a647c1e25a0915dec860d2662eb10fdb382c3c1d1,
      modules: {
        liquidity => LiquidityFunctions: {
          remove_liquidity as RemoveLiquidity => RemoveLiquidityIndexes(),
          close_position as ClosePosition => ClosePositionIndexes(),
        },
      }
  },
}
