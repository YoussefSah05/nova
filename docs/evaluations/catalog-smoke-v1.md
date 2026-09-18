# Initial local catalog smoke evaluation

This is a small synthetic development smoke test, not a model ranking, held-out
benchmark or release qualification. See the [implementation plan](../implementation/catalog-model-evaluation.md).

Runtime: llama.cpp 0.4.1-dev, build 10964, commit b29c606e2, native Darwin arm64.
Host: Mac15,7, 18 GiB unified memory. One model/slot, 4096 context tokens, Metal
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
