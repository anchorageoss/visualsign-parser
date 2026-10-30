# Cosmos SDK wire compatibility

Cosmos transactions are decoded by their signing format and message schema, not by a latest-SDK version gate. An older transaction does not need newly introduced protobuf fields. The v0.55-specific ML-DSA public-key type is additive; it does not restrict classical secp256k1 transactions to v0.55.

The compatibility goal is to retain older valid wire shapes while displaying every understood field. This is not a claim that every SDK module, fork or signing mode is implemented, or that parsing establishes consensus validity or transaction execution.

## Tested schema families

| Tagged SDK sources | Signing/message shape |
| --- | --- |
| v0.30.0 | Legacy Amino `cosmos-sdk/Send` with separate input/output lists; old staking address fields and delegate `value` |
| v0.32.0 | The same bank `Send` name with `from_address`, `to_address`, `amount`; old staking address fields and share-based undelegation |
| v0.33.0 | Modern staking message names/address fields with historical `value` or `shares_amount` |
| v0.34.0 and later | Token-based staking `amount` |
| v0.39.3 | Legacy governance option names `Yes`, `Abstain`, `No`, `NoWithVeto`; current numeric and enum-name forms remain supported |
| v0.40.0 and later | Ordinary protobuf Direct `SignDoc`; bank/staking envelopes and governance v1beta1 |
| v0.46.0 and later | Governance v1 metadata; retained historical `AuthInfo.tip` decoding |
| v0.50.0 / v0.55.0 | Stable ordinary Direct send envelope with newer optional fields absent |
| v0.53.0 | Unordered transactions and protobuf timestamp timeout |

These rows describe schema families, not a trusted version supplied by a coordinator. Compatibility selection uses the complete field shape and registered message type. A chain's semantic activation of a feature cannot be established from transaction bytes alone.

## Safety boundaries

- Historical staking `shares_amount` is displayed as **Delegation shares**, with its original decimal string. It is never converted to tokens or labeled as a token amount. A signed (negative) share amount is refused.
- Governance vote options are accepted as numbers, legacy names (`Yes`, `NoWithVeto`) or enum names (`VOTE_OPTION_YES`). Numeric strings such as `"1"`, which no SDK version emits, are shown as unrecognized.
- Old/new address aliases, coin/share quantities and bank shapes must form one complete, unambiguous schema. Collisions, incomplete shapes, incorrect list/object types and incompatible registered names reject.
- Governance v1beta1 has three vote fields; v1 adds metadata. A legacy vote carrying tag 4 does not become a fully decoded v1 vote: unknown fields inside a modeled protobuf message are refused. Legacy Amino metadata is shown as an unrecognized field.
- Canonical protobuf and Go-style canonical JSON equality checks remain in place, so the document shown is the exact bytes signed. Compatibility never strips or silently rewrites signed fields.
- Unknown transaction-envelope fields still reject. Unknown module messages, extension content and contract-defined execution retain each parser's existing explicit unknown/Partial behavior.
- Tip fields remain visible.
- `SIGN_MODE_DIRECT_AUX`, textual, arbitrary/ADR-36 message signing and EIP-191 preimages are not implemented merely because their names can appear in signer metadata. They need separate explicit request formats and signing support.
- Existing canonical JSON numeric/control-character restrictions remain unchanged. This compatibility work does not introduce new invisible display characters.

## Independent corpus and verification

`src/chain_parsers/visualsign-cosmos/tests/fixtures/sdk-compatibility.json` contains **17 schema-derived fixtures**, referencing **14 tagged official source files** with SHA-256 hashes. The Amino fixtures are canonicalized independently with Go's `encoding/json`; protobuf fixtures are assembled from tagged field numbers without the parser's structs. They are synthetic parsing/display fixtures, not transactions produced by a complete SDK node or claims of on-chain acceptance. Fixture source URLs and hashes are recorded in the JSON itself.

Assertions cover visible semantics, high-precision share text and fixture integrity. Separate regressions in `tests/lib_test.rs` cover malformed/mixed shapes, alias collisions, legacy metadata and unknown nested fields, and assert the specific refusal reason so that an input refused for a different reason (such as non-canonical JSON) does not satisfy the test. Normal test runs require no network access.

To regenerate (Python 3, Go and curl required):

```sh
python3 scripts/generate-cosmos-sdk-fixtures.py \
  --output src/chain_parsers/visualsign-cosmos/tests/fixtures/sdk-compatibility.json
```

Primary references: [v0.30 bank](https://github.com/cosmos/cosmos-sdk/blob/v0.30.0/x/bank/msgs.go), [v0.32 staking](https://github.com/cosmos/cosmos-sdk/blob/v0.32.0/x/staking/types/msg.go), [v0.33 staking](https://github.com/cosmos/cosmos-sdk/blob/v0.33.0/x/staking/types/msg.go), [v0.34 staking](https://github.com/cosmos/cosmos-sdk/blob/v0.34.0/x/staking/types/msg.go), [v0.39 governance](https://github.com/cosmos/cosmos-sdk/blob/v0.39.3/x/gov/types/vote.go), [v0.40 Direct](https://github.com/cosmos/cosmos-sdk/blob/v0.40.0/proto/cosmos/tx/v1beta1/tx.proto), [v0.46 governance](https://github.com/cosmos/cosmos-sdk/blob/v0.46.0/proto/cosmos/gov/v1/tx.proto), [v0.53 transaction fields](https://github.com/cosmos/cosmos-sdk/blob/v0.53.0/proto/cosmos/tx/v1beta1/tx.proto).
