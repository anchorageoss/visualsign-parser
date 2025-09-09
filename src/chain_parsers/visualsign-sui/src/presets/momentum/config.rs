#![allow(dead_code)]

crate::chain_config! {
  config MOMENTUM_CONFIG as Config;

  momentum_mainnet => {
      package_id => 0xcf60a40f45d46fc1e828871a647c1e25a0915dec860d2662eb10fdb382c3c1d1,
      modules as MomentumModules: {
        collect as Collect => CollectFunctions: {
          fee as Fee => FeeIndexes(),
          reward as Reward => RewardIndexes(),
        },
        liquidity as Liquidity => LiquidityFunctions: {
          remove_liquidity as RemoveLiquidity => RemoveLiquidityIndexes(
            liquidity as Liquidity: u128 => 2 => get_liquidity,
            min_amount_x as MinAmountX: u64 => 3 => get_min_amount_x,
            min_amount_y as MinAmountY: u64 => 4 => get_min_amount_y,
          ),
          close_position as ClosePosition => ClosePositionIndexes(),
          add_liquidity as AddLiquidity => AddLiquidityIndexes(
            min_amount_x as MinAmountX: u64 => 4 => get_min_amount_x,
            min_amount_y as MinAmountY: u64 => 5 => get_min_amount_y,
          ),
          open_position as OpenPosition => OpenPositionIndexes(),
        },
        trade as Trade => TradeFunctions: {
          flash_swap as FlashSwap => FlashSwapIndexes(
            is_x_to_y as IsXToY: bool => 1 => get_is_x_to_y,
            exact_input as ExactInput: bool => 2 => get_exact_input,
            amount_specified as AmountSpecified: u64 => 3 => get_amount_specified,
            sqrt_price_limit as SqrtPriceLimit: u128 => 4 => get_sqrt_price_limit,
          ),
          repay_flash_swap as RepayFlashSwap => RepayFlashSwapIndexes(),
          flash_loan as FlashLoan => FlashLoanIndexes(
            amount_x as AmountX: u64 => 1 => get_amount_x,
            amount_y as AmountY: u64 => 2 => get_amount_y,
          ),
          repay_flash_loan as RepayFlashLoan => RepayFlashLoanIndexes(),
          swap_receipt_debts as SwapReceiptDebts => SwapReceiptDebtsIndexes(),
        },
      }
  },
}
