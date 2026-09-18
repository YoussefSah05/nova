# Initial local catalog smoke evaluation

This is a small synthetic development smoke test, not a model ranking, held-out
benchmark or release qualification. See the [implementation plan](../implementation/catalog-model-evaluation.md).

Runtime: llama.cpp 0.4.1-dev, build 10964, commit b29c606e2, native Darwin arm64.
Host: Mac15,7, macOS 26.6.2, 18 GiB unified memory. One model/slot, 4096 context tokens, Metal
offload requested (`-ngl 99`), 512 output-token limit, temperature 0, seed 42,
thinking disabled in request template options. Twelve sequential requests; prompt
caching is enabled by the runtime, so these timings include warmed shared prefixes.
They exclude model download/load and are not independent cold-request latency.

## Qwen3.5-4B Q4_K_M

[Raw report](catalog-qwen35-4b-smoke-v1.json) records exact artifact, prompt, schema,
manifest and corpus hashes. The model and runtime archive hashes were checked
before launch, and the runner verified them again before issuing requests.

- Fully correct: **4/12**.
- Invalid output shape: **0/12**.
- Unsafe identity proposals: **3/12**.
- Missed known identities by abstention: **0/12**.
- Request p50: **1222 ms**; p95: **1618 ms** (12 observations, one pass).

The model guessed an identity for Dune without a year, France 3 without a regional
edition, and a conflicting MBC title/EPG pair. It also frequently omitted explicit
provider quality hints. Valid JSON did not establish safe catalog understanding.
None of these proposals was applied to application data.

Decision: this exact configuration fails the development smoke gate. Keep the
failure as comparison evidence; do not promote it or rewrite expected answers to
make it pass. Run larger candidates against this same prompt/corpus first. Later
prompt/context improvements require new hashes and must be evaluated on held-out
families, rather than repeatedly tuning and claiming generalization on these cases.

No peak-memory, energy, concurrent playback/ASR, model unload, packaged-app or E2E
measurement is established by the report. The local 4B server was stopped after
the run to release resources before the next candidate.

## Same-prompt 9B comparison and versioned diagnosis

Qwen3.5-9B Q4_K_M was downloaded at the pinned revision in its manifest; its
SHA-256 matched before launch. The unchanged v1 prompt scored worse overall:
2/12 fully correct, again three unsafe identity proposals and zero schema failures.
It omitted all explicit quality hints. The runtime's `/apply-template` output was
inspected using the synthetic TF1 input: the system instructions and data were
present, and the non-thinking assistant prefix was rendered. This ruled out missing
instructions in that rendered request, not every possible runtime/model issue.

Prompt v2 explicitly separates quality extraction from identity, identifies title
and group as hint sources, defines missing-discriminator abstention and rejects
title/EPG conflict. It adds no fixture names or expected candidate IDs. Nevertheless,
it was developed after inspecting failures on this corpus, so it is **development
tuning**, not held-out evaluation. v1 remains the runner default and is unchanged.

| Model / prompt | Fully correct | Unsafe links | Missed identities | Schema failures | p50 ms | p95 ms |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| 4B / v1 | 4/12 | 3 | 0 | 0 | 1222 | 1618 |
| 9B / v1 | 2/12 | 3 | 0 | 0 | 2357 | 2976 |
| 9B / v2 | 11/12 | 1 | 0 | 0 | 2426 | 3849 |
| 4B / v2 | 10/12 | 0 | 2 | 0 | 829 | 1737 |

Reports: [9B v1](catalog-qwen35-9b-smoke-v1.json),
[9B v2](catalog-qwen35-9b-smoke-v2.json),
[4B v2](catalog-qwen35-4b-smoke-v2.json). Each row is one pass of 12 cases, not
a stable performance estimate. Shorter abstention outputs partly affect latency;
do not interpret this as a controlled tokens-per-second comparison. Prompt and
schema/corpus hashes in the reports make the variants explicit. The later launches
also restricted CORS to localhost and disabled the web UI; inference settings
remained the same. All benchmark runtime processes were stopped after evaluation.

Remaining v2 errors: 9B chose MBC 2 despite the conflicting MBC 1 EPG ID; 4B
abstained on the identifiable MBC 2 and Arabic-language-name cases. Neither passes
all smoke cases. Larger size did not eliminate unsafe inference. Preserve the
evidence/conflict gate even if subsequent model quality improves.

Decision: **no production selection**. Next compare Ministral with the same
versioned prompts, expand/review the corpus with held-out families, then measure
memory, cancellation and playback/caption coexistence. Any new interpretation
remains Inference evidence until independently corroborated. Neither this report
nor the model writes canonical identities into Nova.
