# Catalog model selection and implementation plan

Status: active development, 2026-09-18. No production model selected.

## Selection policy

The user authorizes choosing a better open model without an arbitrary parameter
count limit. Select for Nova's measured workload and hardware constraints, not
brand, recency or general leaderboard position. The serving app remains local-first;
playback and captions take priority over background catalog inference. Initial
development host: Mac15,7 with 18 GiB unified memory. This is not a fleet minimum.

| Role | Initial comparison | Decision evidence |
| --- | --- | --- |
| Catalog reasoner | Qwen3.5-4B, Qwen3.5-9B, Ministral-3-8B-Instruct-2512 | Structured interpretation, abstention, French/Arabic/English, forbidden links, context conflicts |
| Semantic retrieval | Existing multilingual E5 small vs Qwen3-Embedding-0.6B initially | Recall of correct candidates, multilingual retrieval, index size and rebuild time |
| Reranking | None vs Qwen3-Reranker-0.6B initially | Incremental quality gain per latency/memory cost |

Larger candidates can be added when smaller models fail useful cases and target
hardware can accommodate them. A hardware-dependent model tier is possible, not
yet implemented. Embeddings, reranking and reasoning are different tasks; replacing
E5 with a chat model is not the architecture.

Primary research: [Qwen 9B model card](https://huggingface.co/Qwen/Qwen3.5-9B),
[Qwen 4B](https://huggingface.co/Qwen/Qwen3.5-4B),
[Ministral 8B](https://huggingface.co/mistralai/Ministral-3-8B-Instruct-2512),
[Qwen retrieval models](https://github.com/QwenLM/Qwen3-Embedding), and
[llama.cpp server](https://github.com/ggml-org/llama.cpp/tree/master/tools/server).
Cards establish candidates, not Nova compatibility or measured superiority.
Review exact licenses, converted artifacts and notices before distribution.

## Increment sequence

1. **Development harness (this increment):** 12 synthetic catalog cases, a bounded
   localhost adapter, correctness/false-link/abstention/format metrics, artifact
   checksum checks and deterministic CI tests. Pin the first 4B quantization and
   native runtime. No Python environment required; Node 22 is already pinned.
2. **Real smoke and comparison:** start one model at a time with 4K context and
   bounded output. Record hardware, hashes, configuration and failures. Compare
   quantized 4B, 9B and Ministral on identical requests. Check repeated runs;
   temperature zero is not a cross-runtime determinism guarantee.
3. **Evaluation corpus:** expand beyond smoke into independently reviewed,
   provider/title-family-held-out data, missing fields, hostile text, ambiguous
   aliases, remakes, series/episodes, languages, countries, logos and no-merge pairs.
   This smoke set is visible development data, never a held-out quality claim.
4. **Runtime suitability:** measure cold/warm load, latency distributions, RSS and
   unified-memory pressure, energy, cancellation, offline operation, unload and
   simultaneous playback/ASR/translation. Define acceptance budgets from these
   measurements before selecting a default or tier; download size is not RAM use.
5. **Production integration:** Rust CatalogReasoner/SemanticMatcher interfaces,
   bounded credential-free context, artifact/model management and versioning,
   inference-to-provisional-evidence bridge, cache-first orchestration, research
   cache/verified knowledge, canonicalization and shadow import validation.

The core evidence evaluator introduced in ADR 0008 only evaluates registered
assertions. The development reasoner produces **unverified interpretations**;
its output is not passed off as trusted evidence or written to the catalog.
Candidate IDs refer only to supplied synthetic candidates. A future bridge must
register novel interpretations at Inference authority, preserve input provenance
and obtain corroboration before stronger claims. Successful smoke does not bypass
identity/cannot-link policy, raw-source integration or app E2E gates.

## Reproduction

From the repository root, `pnpm test:catalog-eval` runs eight tests with a synthetic
loopback HTTP server and needs no model, token or network access beyond localhost.
CI runs it beside tooling tests. `pnpm eval:catalog` uses a separately started local
runtime. It never launches/downloads a model or accesses user profiles itself.

First artifact: `tools/catalog-eval/qwen35-4b.manifest.json`. The 2.74 GB community
conversion is pinned by repository revision and Hub LFS SHA-256. llama.cpp v0.4.1
maps to build b10964; its macOS archive digest comes from the upstream release API.
Models/runtime files stay under ignored `models/catalog-eval/`.

```sh
hf download unsloth/Qwen3.5-4B-GGUF Qwen3.5-4B-Q4_K_M.gguf \
  --revision e87f176479d0855a907a41277aca2f8ee7a09523 \
  --local-dir models/catalog-eval/qwen35-4b --max-workers 1

# After checking the downloaded archive against the manifest and extracting it:
models/catalog-eval/runtime/llama-b10964/llama-server \
  -m models/catalog-eval/qwen35-4b/Qwen3.5-4B-Q4_K_M.gguf \
  --alias nova-catalog-eval --host 127.0.0.1 --port 18491 \
  -c 4096 -np 1 -ngl 99 --jinja --cors-origins localhost --no-webui

pnpm eval:catalog --manifest tools/catalog-eval/qwen35-4b.manifest.json \
  --output /tmp/nova-catalog-smoke.json
```

Runner checks on-disk artifacts before inference, hashes corpus/prompt/schema and
manifest, uses one request at a time, rejects redirects and limits requests to
45 seconds/512 output tokens/64 KiB response. Expected answers never enter the
request. Each report distinguishes runtime/format failures from incorrect answers.
Exit 0 means all smoke cases passed, 2 means evaluation misses, 1 means setup error.
Report files use exclusive creation to avoid overwriting prior results.

The process serving the port is not cryptographically attested by the runner;
operators must launch the verified runtime/model and record its command/log. Memory
and playback interference are explicitly unmeasured until separately instrumented.
This is a development interface, not a production model service or HTTP IPC design.

## Initial runtime verification

The downloaded macOS archive matched its pinned SHA-256. The extracted server
reports `0.4.1-dev (build 10964, commit b29c606e2)`, AppleClang 21, Darwin arm64.
The upstream release tag is v0.4.1; record the binary's actual dev version as above
rather than assuming its reported version matches the release label.

First actual inference result: Qwen3.5-4B Q4_K_M produced 4/12 fully correct
responses and three unsafe links. It is not selected. See the
[smoke report](../evaluations/catalog-smoke-v1.md). Qwen3.5-9B Q4_K_M is pinned in
`tools/catalog-eval/qwen35-9b.manifest.json` for the unchanged-prompt comparison.

Completed comparison and diagnosis: 9B/v1 scored 2/12 with three unsafe links.
The separately versioned diagnostic prompt v2 scored 11/12 with one unsafe link
on 9B, and 10/12 with zero unsafe links but two missed identities on 4B. Pass
`--prompt-version v2` to reproduce the diagnostic; v1 remains the default. All
four raw reports are retained. No production default is selected. Next: Ministral,
held-out multilingual review, resource/coexistence testing, then the Rust adapter
and provisional-evidence bridge. Prompt tuning here is not held-out validation.
