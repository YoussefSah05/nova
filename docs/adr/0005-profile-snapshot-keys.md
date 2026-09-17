# ADR 0005: provision and load profile snapshot keys separately

Status: lifecycle contract tested; macOS adapter compiled; native provisioning
blocked by MissingEntitlement in the Cargo test executable. Not wired into imports.
Date: 2026-09-17.

## Problem and decision

Replacing a snapshot encryption key makes existing evidence unreadable. A missing
key during replay must mean recovery is needed, not permission to create a new one.
`ProfileKeyVault::load` is read-only. `provision` generates 32 random bytes and uses
an atomic insert-only backend; an existing item returns AlreadyExists without
modification. Concurrent callers get one winner, not two different active keys.
Only new profiles/key namespaces may provision. The future profile coordinator
must enforce that rule using durable lifecycle state; the vault cannot infer whether
a profile has retained files. Existing profile identifiers need a stable UUID mapping.

Read failures distinguish Missing, InvalidKey, MissingEntitlement and Unavailable.
Authentication, locked access, cancellation and unknown OS errors never become
Missing and never trigger automatic regeneration. No replacement, deletion, rotation,
export or fallback API is offered. A cancelled await can still leave a key created
by its blocking worker; load resolves that uncertainty. There is no key-file fallback.

## Native adapter and research

The default namespace is `app.nova.desktop.snapshot-key.v1`, with a profile UUID
as account. Native operations run on Tokio blocking workers. Key material is never
logged or formatted; owned read/generation buffers are zeroized. Framework/OS and
caller copies are not covered by that erasure guarantee.

The existing security-framework 3.7.0 password setter is an upsert, so use its
[ItemAddOptions](https://docs.rs/security-framework/3.7.0/security_framework/item/struct.ItemAddOptions.html)
SecItemAdd wrapper instead. Read with ItemSearchOptions scoped to generic passwords,
exact service/account, one data result and non-synchronized items. Enable the
OSX_10_15 feature for explicit data-protection Keychain selection. Existing
core-foundation 0.10.1 supplies CFData. Cargo.lock gains direct dependency edges
only; there are no package upgrades and no handwritten unsafe FFI.

Apple recommends the [data-protection Keychain](https://developer.apple.com/documentation/security/ksecusedataprotectionkeychain)
for these operations. Creation omits the synchronization attribute, which Apple
[defines as non-synchronized](https://developer.apple.com/documentation/security/ksecattrsynchronizable);
reads explicitly select non-synchronized data. Apple's default accessibility is
[WhenUnlocked](https://developer.apple.com/documentation/security/ksecattraccessiblewhenunlocked).
This permits encrypted-backup migration; it is not a ThisDeviceOnly policy and must
remain consistent with future snapshot backup/recovery design. No access group is
invented: the signed application's default group is used. There is no legacy-keychain
fallback when signing or access fails.

## Validation and observed limitation

Six backend contract tests cover absent reads, malformed material, insert-only
provisioning, distinct profile keys, concurrent callers, access failure and encrypted
fixture replay after vault recreation. Tests were written first and failed to compile
against the missing contract. A seventh test verifies native error classification.
These tests use an in-memory backend and do not establish native Keychain durability.

An explicit ignored native roundtrip test uses a fresh synthetic namespace and
exact-item cleanup. It reads an absent item, provisions, loads and checks that a
second provision cannot replace the key, then deletes only its own test item.
Run deliberately on an appropriately signed test host:

```sh
cargo test -p nova-storage --lib profile_keys::native::native_keychain_roundtrip \
  --locked -- --ignored --exact
```

On this host the initial missing-item read completed, but native provisioning failed
with MissingEntitlement. This was an OS rejection, not a successful native roundtrip
and not a sandbox approval rejection. The data-protection Keychain derives permitted
access groups from signing entitlements; see Apple's [TN3137](https://developer.apple.com/documentation/technotes/tn3137-on-mac-keychains)
and [MissingEntitlement guidance](https://developer.apple.com/documentation/security/errsecmissingentitlement).
Do not suppress this failure, mark the native test passing, invent team identifiers,
or switch to weaker storage simply to make the unsigned test executable pass.

The ordinary workspace suite deliberately excludes this native test; explicit
execution is a mandatory integration gate. Signed app-level provisioning, restart,
locked/unlocked behavior, denial and recovery still require validation. No app profile
or existing user credential was read or changed by the synthetic probe.

## Next and rollback

Next: validate native operations from the signed app, implement durable new-profile
provisioning/recovery, then streaming/aggregate snapshot retention and lossless adapters.
Deletion/rotation requires explicit reference accounting and a versioned key identifier
in the storage format before implementation. No automatic deletion of a key whose
snapshots may still be retained. Rollback removes this unconsumed module/dependency
edges; no profile migration, production key or ciphertext format was changed.
