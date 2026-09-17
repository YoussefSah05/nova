# ADR 0002: preserve provider evidence behind typed source contracts

Status: implemented on the catalog source-contract branch; not integrated into main.
Date: 2026-09-17.

## Context

Existing ParsedEntry and storage structs mix provider input, derived titles, row IDs
and playable URLs. Changing every consumer together would risk imports and user state.
The production blueprint separates provider evidence, canonical identity and variants.

## Decision

Add a pure `nova-core::catalog` module first, retaining all existing APIs. Introduce
typed opaque IDs, a bounded immutable SourceRecord and a distinct StreamVariant view.
Preserve raw multilingual strings exactly; model unknown kinds/numbers explicitly.
Do not derive source serialization or expose it through IPC. Diagnostics summarize
safe opaque IDs/counts and never print raw provider fields or protected locators.

No identity matching or ID allocation algorithm is selected in this increment.
The previous blueprint's research on versions/external identities informs the boundary;
resolver rules and SQLite publication still require their own evidence and ADRs.

## Alternatives and consequences

Changing legacy ParsedEntry would force a broad parser/storage/UI migration too soon.
Using bare strings for every ID permits accidental entity/variant confusion. A new
crate per type adds unnecessary workspace complexity; the existing core crate is
sufficient for this pure contract. Serialization of raw provider evidence waits for
the protected snapshot storage design instead of accidentally creating a public DTO.

These contracts are deliberately not wired into production. No behavior or user data
changes; removal is a safe rollback. The module can move to a catalog crate when
compiler responsibilities justify it. Tests cover limits, redaction, preservation and
typed ID constraints. This is a foundation slice, not completion of Workstream 1.
