# MMX wallet helpers for Stainless

This package is implemented entirely in `src/wallet.stl`. It provides MMX
wallet and poker protocol helpers to Stainless applications:

- wallet identity, signature, and address operations;
- canonical `poker2.js` seed, commitment, action, checkpoint, continuation,
  and dealer-authentication hashes implemented in `wallet.stl`.

There is no Rust source in this package or in `stainless-crypto`. Integer
encoding and exact wide-ratio arithmetic are implemented in Stainless.
`sha256.stl` and `secp256k1.stl` expose Stainless facades that bind directly to
the established `sha256` and `secp256k1` crates; there is no custom Rust crypto
wrapper or wallet ABI. The poker dealer compiles these sources directly into
its Stainless translation unit.

The package deliberately does not store wallet secrets. The poker dealer uses
the MMX node wallet WAPI for deployment and settlement signing; these helpers
support player clients and verify off-chain protocol messages.
