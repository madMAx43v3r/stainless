# MMX wallet helpers for Stainless

This package is implemented entirely in `src/wallet.stl`. It provides generic
MMX wallet helpers to Stainless applications:

- wallet identity, signature, and address operations;
- text and hexadecimal SHA-256 helpers.

There is no Rust source in this package or in `stainless-crypto`. Integer
encoding and exact wide-ratio arithmetic are implemented in Stainless.
`sha256.stl` and `secp256k1.stl` expose Stainless facades that bind directly to
the established `sha256` and `secp256k1` crates; there is no custom Rust crypto
wrapper or wallet ABI. The poker dealer compiles these sources directly into
its Stainless translation unit.

The package deliberately does not store wallet secrets. Applications can use
it for local player signatures and address verification, while services can
continue using the MMX node wallet WAPI for managed signing.

Run its Stainless tests from the workspace root with:

```sh
stainlessc --run --package apps/mmx-wallet/test
```
