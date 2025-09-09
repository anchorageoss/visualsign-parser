#![allow(dead_code)]

crate::chain_config! {
  config NATIVE_STAKING_CONFIG as Config;

  native_staking => {
      package_id => 0x3,
      modules as NativeStakingModules: {
        sui_system as SuiSystem => SuiSystemFunctions: {
          request_add_stake as AddStake => AddStakeIndexes(),
          request_withdraw_stake as WithdrawStake => WithdrawStakeIndexes(),
        },
      }
  },
}
