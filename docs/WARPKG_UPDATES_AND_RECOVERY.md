# WarPkg Offline Updates and Recovery

WarShield Pass 4 adds a narrow operational update model on top of the existing signed-bundle verification path.

## Current model

- Updates are explicit local operations.
- A bundle is either staged with `warpkg stage <bundle-path>` or applied directly with `warpkg apply <bundle-path>`.
- `warpkg apply staged` applies the currently staged bundle.
- The same signed-bundle verification path used by `warpkg verify` is reused before any offline apply.
- Unsigned, tampered, or otherwise invalid bundles are rejected deterministically.

## Persisted state

The kernel persists current update and recovery metadata in:

- `/var/pkg/update-state.json`
- `/var/pkg/staged/*`
- `/var/pkg/rollback/*`

The current state tracks:

- whether a bundle is only staged
- whether an applied update is pending confirmation
- whether the post-update boot reached shell-ready
- whether the update was confirmed, failed, or rolled back
- whether recovery has been requested explicitly or automatically

## Boot-health semantics

- After an offline apply, the next boot enters `pending-confirmation`.
- When boot reaches shell-ready, the kernel records that observation.
- An operator must then confirm the boot with `warpkg confirm` or `recovery confirm`.
- If the next boot occurs without that confirmation, the kernel marks the update failed and requests recovery.
- If boot never reaches shell-ready, the next boot also marks the update failed and requests recovery.

## Recovery semantics

Recovery is intentionally small and honest:

- it is a privileged shell entry path, not a separate rescue OS
- it can inspect update state with `recovery status`
- it can confirm, reject, or roll back the current update
- it can clear a manual recovery request with `recovery resume` when no pending/failed update blocks that action

Recovery can be requested either automatically by update-health logic or manually through `recovery enter` / `reboot recovery`.

## Reviewable proof path

`warpkg proof` runs a controlled local self-test that:

- applies a valid signed local bundle through the Pass 4 offline apply path
- advances that state into pending confirmation
- records shell-ready observation
- confirms the update
- restores the rollback snapshot
- proves that the recovery entry/status path becomes observable when requested
- rejects a tampered offline bundle deterministically

The proof intentionally refuses to run while a real update or recovery request is already active.

When the persisted update state is idle, boot now runs the same proof path automatically and
emits deterministic `[PROOF]` markers for offline apply, pending confirmation, shell-ready
observation, confirmation, rollback restoration, recovery observability, and tamper rejection.

## Current limits

- no remote update service
- no trust-root rotation or revocation
- no secure-boot linkage
- no A/B slot switching
- no whole-system snapshot manager
- rollback is limited to the current package-apply snapshot path

This is deliberate. Pass 4 is a resilience foundation for controlled deployment and hardware-real preparation, not a full production update platform.
