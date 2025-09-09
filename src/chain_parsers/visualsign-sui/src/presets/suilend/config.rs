#![allow(dead_code)]

crate::chain_config! {
  config SUILEND_CONFIG as Config;

  suilend_mainnet => {
      package_id => 0xdf6df8a1e16c58b66d5c432cc6a5a8981c9982f111723ec0894d152be57b3e7e,
      modules as SuiLendModules: {
        lending_market as LendingMarket => LendingMarketFunctions: {
          borrow_request as BorrowRequest => BorrowRequestIndexes(
                reserve_array_index as ReserveArrayIndex: u64 => 1 => get_reserve_array_index,
                amount as Amount: u64 => 4 => get_amount,
          ),
          claim_rewards as ClaimRewards => ClaimRewardsIndexes(
                reserve_id as ReserveId: u64 => 3 => get_reserve_id,
                reward_index as RewardIndex: u64 => 4 => get_reward_index,
                is_deposit_reward as IsDepositReward: bool => 5 => get_is_deposit_reward,
          ),
          claim_rewards_and_deposit as ClaimRewardsAndDeposit => ClaimRewardsAndDepositIndexes(
                reward_reserve_id as RewardReserveId: u64 => 3 => get_reward_reserve_id,
                reward_index as RewardIndex: u64 => 4 => get_reward_index,
                is_deposit_reward as IsDepositReward: bool => 5 => get_is_deposit_reward,
                deposit_reserve_id as DepositReserveId: u64 => 6 => get_deposit_reserve_id,
          ),
          create_obligation as CreateObligation => CreateObligationIndexes(),
          deposit_ctokens_into_obligation as DepositCTokensIntoObligation => DepositCTokensIntoObligationIndexes(
                reserve_array_index as ReserveArrayIndex: u64 => 1 => get_reserve_array_index,
          ),
          deposit_liquidity_and_mint_ctokens as DepositLiquidityAndMintCTokens => DepositLiquidityAndMintCTokensIndexes(
                reserve_array_index as ReserveArrayIndex: u64 => 1 => get_reserve_array_index,
          ),
          fulfill_liquidity_request as FulfillLiquidityRequest => FulfillLiquidityRequestIndexes(
                reserve_array_index as ReserveArrayIndex: u64 => 1 => get_reserve_array_index,
          ),
          rebalance_staker as RebalanceStaker => RebalanceStakerIndexes(
                sui_reserve_array_index as SuiReserveArrayIndex: u64 => 1 => get_sui_reserve_array_index,
          ),
          redeem_ctokens_and_withdraw_liquidity_request as RedeemCTokensAndWithdrawLiquidityRequest => RedeemCTokensAndWithdrawLiquidityRequestIndexes(
                reserve_array_index as ReserveArrayIndex: u64 => 1 => get_reserve_array_index,
          ),
          refresh_reserve_price as RefreshReservePrice => RefreshReservePriceIndexes(
                reserve_array_index as ReserveArrayIndex: u64 => 1 => get_reserve_array_index,
          ),
          repay as Repay => RepayIndexes(
                reserve_array_index as ReserveArrayIndex: u64 => 1 => get_reserve_array_index,
          ),
          unstake_sui_from_staker as UnstakeSuiFromStaker => UnstakeSuiFromStakerIndexes(
                sui_reserve_array_index as SuiReserveArrayIndex: u64 => 1 => get_sui_reserve_array_index,
          ),
          withdraw_ctokens as WithdrawCTokens => WithdrawCTokensIndexes(
                reserve_array_index as ReserveArrayIndex: u64 => 1 => get_reserve_array_index,
                amount as Amount: u64 => 4 => get_amount,
          ),
        },
      }
  },
}
