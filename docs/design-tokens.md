# Design Tokens

> Source of truth for design tokens is `apps/desktop/src/styles/globals.css` (@theme block).
> **Status**: Phase 0 stub.

## Type scale

| Token | Value | Usage |
|---|---|---|
| `text-xs` | 12px | Labels, metadata |
| `text-sm` | 14px | Secondary text, captions UI |
| `text-base` | 16px | Body, form inputs |
| `text-lg` | 18px | Subheadings |
| `text-xl` | 22px | Translation card primary text |
| `text-2xl` | 28px | Section headings |
| `text-3xl` | 36px | Page headings |

Weights: **400** (regular) and **500** (medium) only.

## Color palette (dark-first)

| Token | Value | Usage |
|---|---|---|
| `surface` | `#0a0a0c` | App background (never pure black) |
| `surface-raised` | `#111113` | Cards, panels |
| `surface-overlay` | `#1a1a1e` | Popovers, dropdowns |
| `accent` | `#6366f1` | Primary interactive elements |
| `text-primary` | `rgba(255,255,255,0.96)` | Main content |
| `text-secondary` | `rgba(255,255,255,0.55)` | Supporting text |
| `text-muted` | `rgba(255,255,255,0.30)` | Placeholders, disabled |
| `border` | `rgba(255,255,255,0.08)` | Dividers, card edges |

## Motion

| Token | Value | Usage |
|---|---|---|
| `ease-enter` | `cubic-bezier(0.16, 1, 0.3, 1)` | Elements appearing |
| `ease-exit` | `cubic-bezier(0.7, 0, 0.84, 0)` | Elements disappearing |
| `duration-fast` | 150ms | Exits, micro-interactions |
| `duration-default` | 200ms | Standard transitions |
| `duration-layout` | 400ms | Layout shifts, panels |

## Translation card (middle-box, Mode B/C)

See ARCHITECTURE.md §10 for full spec.
Background: `rgba(10, 10, 12, 0.78)` + `backdrop-filter: blur(20px) saturate(160%)`.
