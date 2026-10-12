# ADR 0013: Web interface on Tauri, with native video and a Rust core

- **Status:** Accepted (recorded 2026-10-12; the choice predates this record)
- **Deciders:** Nova maintainer
- **Supersedes:** none. ADR 0012 is reserved for the playback audio tap.

## Context

Nova is a macOS-first media application with three very different workloads:

1. a catalog of tens of thousands of provider entries, imported, cleaned, searched and enriched;
2. video playback across arbitrary codecs and containers, including live streams;
3. on-device AI (speech recognition, translation, embeddings) that must never disturb playback.

The interface is ambitious: cinematic layouts, motion, a programme guide, and fast iteration on
design. The question is which technology draws that interface. The choice was made early and
never written down; it was questioned after frame-rate measurements showed WebKit's 60 fps cap
on ProMotion displays.

## Options

| Option | For | Against |
|---|---|---|
| **Native (SwiftUI/AppKit)** | 120 Hz and system behaviour for free, best energy use, native accessibility | macOS only; slower design iteration; a full rewrite of the interface |
| **Electron** | Chromium: 120 Hz, mature tooling | Bundles a browser: high baseline memory and size, rejected in the original brief |
| **Tauri (system WebKit) + React** | Small and light, Rust backend, fast UI iteration, shared code if other platforms come | WebKit quirks, 60 fps cap by default, a native/web bridge for video and accessibility |

## Decision

Draw the interface with React in Tauri's system WebKit view, and keep everything
performance-critical out of it:

- **Video** is rendered by libmpv into a native Core Animation layer under the webview.
- **Catalog, playback policy, storage, networking and AI** run in Rust.
- The webview renders state it receives over typed IPC and sends intents back.

A full SwiftUI rewrite and a move to Electron are explicitly out of scope (see the archived
[Player 2.0 plan](../archive/nova-player-2-technical-plan.md) and
[production blueprint](../archive/nova-production-architecture-blueprint.md)).

## Consequences

- Interface performance needs deliberate care: virtualized lists, no CSS filters on repeated
  elements, nothing optional while scrolling (see [Performance](../architecture/performance.md)).
- WebKit's 60 fps pacing is lifted through its feature list, with a setting to restore it. The
  hook is an internal WebKit interface: if a release removes it, Nova falls back to 60 fps.
- Accessibility spans a web tree and native layers and must be tested with VoiceOver.
- If a premium native feel is wanted later, the path is selective native pieces (player
  controls, menus, window chrome), not a rewrite.
