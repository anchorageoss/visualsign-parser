---
category: ethereum
label: "Universal Router commands with allow-revert are shown"
description: ""
tags: ["Wallet API"]
---
Every command byte of a Uniswap Universal Router `execute` call is now listed.
Before, a command with the allow-revert bit (`0x80`) was dropped from the
display, so a `PERMIT2_PERMIT` sent as `0x8a` was not shown. Such a command now
carries the suffix " (allow revert)" on its title and in the summary line. A
command type the parser has no name for is listed as "Unknown command 0x.."
with its input hex, where it was skipped before. A command the parser names but
has no decoder for (`V4Swap`, `AcrossV4DepositV3`) is listed by its name with
its input as hex.

The router masks each byte with a `COMMAND_TYPE_MASK` that depends on its
version, and the parser now does the same. The V1.2 and V2.0 routers use `0x3f`,
the V2.1.1 and V2.2.0 routers use `0x7f`. On V2.1.1 and V2.2.0, `0x40` is
`ACROSS_V4_DEPOSIT_V3` (a bridge deposit) and is named `AcrossV4DepositV3`, with
its input as hex. On the V1.2 routers, `0x10` to `0x14` are other commands than
the V4 and position-manager commands the parser names, so they are shown as
unknown. An unlisted router never gets `0x40` named. A wallet that matches
command titles must expect the new " (allow revert)" suffix and the new unknown
entries.
