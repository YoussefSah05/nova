# Catalog intelligence baseline — 2026-09-18

Starting revision: `766bae9`, clean `feat/protected-record-staging`. This increment
uses `feat/catalog-intelligence-evidence`. Historical `v0.1.0` and `main` are
preserved; development version remains `0.2.0-alpha.0`. Migration branches are
stacked locally, not evidence of green remote CI or completed integration.

| Area | Code evidence and current state |
| --- | --- |
| Shell/frontend | `apps/desktop/package.json`: React 19/TypeScript/Vite; `src-tauri/tauri.conf.json`: Tauri 2 app, macOS 14 minimum. No UI changes in this slice. |
| Import | `src-tauri/src/commands/mod.rs` calls existing M3U/Xtream clients and writes legacy repositories. `crates/m3u/src/lib.rs`, `crates/core/src/clean.rs` and `classify.rs` still derive titles/categories using existing rules. |
| Grouping/enrichment | `commands/enrich.rs` uses dedup keys, embeddings and TMDB. `crates/enrich/src/embed.rs` selects multilingual E5 small through fastembed/ONNX. No catalog LLM/research-agent runtime is present. |
| Persistence/search | SQLx/SQLite profile migrations through 0019; existing grouping, TMDB caches, embeddings and transcript FTS. `crates/search` supplies FTS query handling. No canonical research-cache migration yet. |
| Playback/AI | Existing libmpv/native view integration; whisper-cli subprocess ASR; NLLB-200 via CTranslate2 CPU. These are separate from catalog reasoning. |
| New source foundation | `nova-core::catalog`, protected streaming snapshots and `nova-catalog-ingest` preserve evidence. The application does not consume the new ingest crate yet. |
| CI/dependencies | Existing macOS Rust and Linux frontend checks in `.github/workflows/ci.yml`; locked Cargo/pnpm dependencies and `nova-stack.json`. No dependency upgrades/model downloads in this increment. |
| Validation baseline | Previous checkpoint: 221 passing Rust tests; subprocess helper excluded from direct discovery but called by parent tests; native Keychain provisioning previously failed MissingEntitlement. No native E2E/real-provider/40k benchmark proof. |

The revised request supersedes the blueprint's optional catalog-LLM framing.
Intelligent understanding is required; choice of reasoning model remains measured.
The first new slice adds executable proposal/evidence validation, not an agent
disguised as a regex cleaner. See ADR 0008 and the progress ledger for its limits.

Repository/code/history inspection informs this baseline. No new running-app
inspection was performed, and existing UI/media capability is not fresh E2E proof.
