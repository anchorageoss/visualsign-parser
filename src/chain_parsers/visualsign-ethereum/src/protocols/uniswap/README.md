# Uniswap Protocol

## Contracts

| Contract              | Address                                      |
|-----------------------|----------------------------------------------|
| Universal Router V1.2 | `0x3fC91A3afd70395Cd496C647d5a6CC9D4B2b7FAD` |
| Permit2               | `0x000000000022D473030F116dDEE9F6B43aC78BA3` |

## Networks

| Chain    | ID    |
|----------|-------|
| Ethereum | 1     |
| Optimism | 10    |
| Polygon  | 137   |
| Base     | 8453  |
| Arbitrum | 42161 |

## Universal Router Commands

Reference: [Dispatcher.sol](https://github.com/Uniswap/universal-router/blob/main/contracts/base/Dispatcher.sol)

| Cmd  | Name                        | Parameters (Solidity)                                   | Status  |
| ---- | --------------------------- | ------------------------------------------------------- | ------- |
| 0x00 | V3_SWAP_EXACT_IN            | `(address, uint256, uint256, bytes path, bool)`         | Custom  |
| 0x01 | V3_SWAP_EXACT_OUT           | `(address, uint256, uint256, bytes path, bool)`         | Custom  |
| 0x02 | PERMIT2_TRANSFER_FROM       | `(address, address, uint160)`                           | Custom  |
| 0x03 | PERMIT2_PERMIT_BATCH        | `(PermitBatch, bytes)`                                  | -       |
| 0x04 | SWEEP                       | `(address token, address recipient, uint256 amountMin)` | Custom  |
| 0x05 | TRANSFER                    | `(address, address, uint256)`                           | Custom  |
| 0x06 | PAY_PORTION                 | `(address, address, uint256 bips)`                      | Custom  |
| 0x08 | V2_SWAP_EXACT_IN            | `(address, uint256, uint256, address[] path, address)`  | Custom  |
| 0x09 | V2_SWAP_EXACT_OUT           | `(uint256, uint256, address[] path, address)`           | Custom  |
| 0x0A | PERMIT2_PERMIT              | `(PermitSingle, bytes sig)`                             | Custom  |
| 0x0B | WRAP_ETH                    | `(address recipient, uint256 amountMin)`                | Custom  |
| 0x0C | UNWRAP_WETH                 | `(address, uint256)`                                    | Custom  |
| 0x0D | PERMIT2_TRANSFER_FROM_BATCH | `(AllowanceTransferDetails[])`                          | -       |
| 0x0E | BALANCE_CHECK_ERC20         | `(address, address, uint256)`                           | -       |
| 0x10 | V4_SWAP                     | `(bytes)`                                               | -       |
| 0x11 | V3_POSITION_MANAGER_PERMIT  | `(bytes)`                                               | Default |
| 0x12 | V3_POSITION_MANAGER_CALL    | `(bytes)`                                               | Default |
| 0x13 | V4_INITIALIZE_POOL          | `(PoolKey, uint160)`                                    | -       |
| 0x14 | V4_POSITION_MANAGER_CALL    | `(bytes)`                                               | Default |
| 0x21 | EXECUTE_SUB_PLAN            | `(bytes commands, bytes[] inputs)`                      | -       |

**Status:** `Custom` = human-readable, `Default` = raw hex, `-` = not implemented
