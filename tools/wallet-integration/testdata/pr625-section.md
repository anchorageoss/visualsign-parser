## Summary

Solana intermediate output moves to schema v5 with a trailing `max_fee_lamports`.

## Wallet integration

`SOLANA_INTERMEDIATE_SCHEMA_VERSION` goes from 4 to 5, and `SolanaMetadata` gains
`max_fee_lamports`, which changes `metadata_digest` for Solana requests that carry
`chain_metadata`.

- requires: Anchorage HSM Solana intermediate reader >= schema v5 reader
- requires: visualsign-turnkeyclient >= first release with anchorageoss/visualsign-turnkeyclient#47
- deploy-order: both readers ship first; deploy the parser only after they are live

## Testing

- requires: outside the section, ignored >= 0
