# Sui decoder compatibility fixtures

`unsupported-sdk-transactions.json` contains base64 BCS `Transaction` values for
three encodings added after the pre-migration `mainnet-v1.68.1` type definitions:
`Validity`, a `SenderAllowance` withdrawal with distinct funder/allowance addresses,
and `ForwardingAddressRegistryCreate`. They use a deterministic sender/gas owner
(`0x01` repeated 32 times), gas price 1 and budget 1000. Tests require the SDK to
decode each complete transaction, the old types to reject it, and conversion to
fail before emitting a preview or intermediate output.

The signature fixture subdirectory records provenance for the upstream SDK's
real authentication fixtures; these test parsing compatibility only, not signature
verification.
