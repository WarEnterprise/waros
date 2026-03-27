# WarPkg Signing

WarShield Pass 2 gives WarPkg a narrow but real end-to-end signature path.

## Current design

- Package bundles are serialized as JSON `WarPackBundle` values.
- Each bundle carries a `signed_manifest` and one or more payloads.
- The signed manifest covers package identity metadata plus the expected payload digests and sizes.
- The signature scheme is `ml-dsa-dilithium3-v1`.
- The current trust root is one embedded bootstrap key: `waros-bootstrap-root-v1`.

## What is signed

The current signed manifest binds:

- package name
- version
- description
- dependency list
- one entry per payload with:
  - path
  - byte length
  - SHA3-256 digest

The package index separately records a transport SHA-256 digest of the serialized bundle.
WarPkg verifies both layers before install or apply.

## Verification order

`kernel/src/pkg/mod.rs` currently verifies packages in this order:

1. the serialized bundle bytes must match the package-index SHA-256 digest
2. the bundle must deserialize as a `WarPackBundle`
3. the signed manifest must be present
4. the signature must verify against the embedded bootstrap ML-DSA trust root
5. every payload digest and size in the signed manifest must match the payload bytes
6. the signed manifest metadata must match the package-index metadata used for installation

If any step fails, installation is rejected deterministically.

## What is rejected today

- unsigned bundles
- bundles with an invalid signature
- bundles whose payload bytes do not match the signed manifest
- bundles whose transport digest does not match the package index
- bundles whose signed metadata does not match the package-index entry

## Trust-root scope

This is intentionally a small trust model:

- one embedded bootstrap trust root
- no delegated repository metadata
- no key rotation
- no revocation
- no external keyring management
- no secure-boot linkage

That scope is deliberate. WarShield Pass 2 closes the placeholder-grade package verification gap without redesigning the full package ecosystem.
