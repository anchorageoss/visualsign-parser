---
category: core
label: "Added Cosmos SDK sign doc parser"
description: ""
tags: ["Wallet API"]
---
`parser_cli` and the parser app now decode Cosmos SDK sign docs in direct (protobuf)
and Amino JSON modes: bank send, staking, distribution, gov vote, authz exec, IBC
transfer, and CosmWasm execute. See [Cosmos](/chains/cosmos).
