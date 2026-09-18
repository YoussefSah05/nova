# Synthetic transfer protocol — frozen before first model run

The initial smoke corpus was used to refine prompt v2. It is development data.
`fixtures/catalog-intelligence/v1/transfer.json` adds 12 cases with disjoint title
families and six expected abstentions. Its fictional names and explicit candidate
metadata test interpretation rather than memorized broadcaster knowledge. Coverage
adds episode/season ambiguity, source identifier conflict, country-specific
editions, blank names and an injection attempt with an available wrong candidate.

The corpus is authored by the same implementation agent with knowledge of the
prompt. It is not independently reviewed, statistically representative, or proof
that the base models never encountered analogous examples. Freeze it in Git before
inference and do not silently change expected answers after seeing results.

Run prompt v2 unchanged on Qwen 4B, Qwen 9B and Ministral 8B with the existing
runtime, quantization manifests and limits. Use `--corpus` to select this set and
retain a separate report for each model. Review semantic mistakes, unsafe links,
abstention, format failures and latency separately. If an expected answer is later
found wrong, document the correction, version the corpus and rerun all candidates.

This is a transfer smoke step before independently reviewed held-out/provider
data. It cannot qualify a model for production by itself. Memory, cancellation,
playback coexistence and the canonical evidence gate remain separate requirements.

## Recorded runs

The corpus was committed at `2ed003c` before any model saw it. Prompt v2, output
schema, sampling and runtime limits remain unchanged. These are one-pass synthetic
transfer measurements on the same Mac15,7/18 GiB host, not held-out validation.

| Model | Correct | Unsafe links | Missed identities | Schema failures | p50 ms | p95 ms |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Qwen3.5-4B Q4_K_M | 11/12 | 0 | 1 | 0 | 891 | 1813 |
| Qwen3.5-9B Q4_K_M | 11/12 | 1 | 0 | 0 | 2363 | 3981 |
| Ministral 3 8B Q4_K_M | 9/12 | 3 | 0 | 0 | 2259 | 2533 |

Raw reports: [4B](catalog-qwen35-4b-transfer-v2.json),
[9B](catalog-qwen35-9b-transfer-v2.json). The 4B model abstained on the remake
despite a supplied year. The 9B model ignored the conflicting Arabic channel EPG
identifier and selected the title's candidate. Both models' runtimes were stopped
after their runs. Neither run writes to the app or qualifies a production model.

[Ministral report](catalog-ministral3-8b-transfer-v2.json): it selected identities
despite an EPG conflict, a missing remake year, and an instruction embedded in the
provider group. Its runtime was also stopped. The injection failure specifically
supports keeping model interpretations untrusted and excluding direct catalog writes.

Combining the original development set and this transfer set under prompt v2:

| Model | Fully correct | Unsafe links | Missed identities |
| --- | ---: | ---: | ---: |
| Qwen3.5-4B | 21/24 | 0 | 3 |
| Qwen3.5-9B | 22/24 | 2 | 0 |
| Ministral 3 8B | 17/24 | 4 | 2 |

This aggregation is descriptive, not a population accuracy estimate. The original
set influenced prompt v2 and the transfer set lacks independent review. The
Ministral totals also contain one candidate-name contract failure.

Development decision: use **Qwen3.5-4B Q4_K_M with prompt v2** as the reference
for the next Rust inference/provisional-evidence adapter. Prefer its observed
abstentions to unsupported identity selection at this stage. This is not a
production model selection or an automatic entity-merge policy. Its three missed
identities require better contextual evidence or a later verified enrichment pass.
Keep the other pinned candidates for future comparisons; no weights were deleted.
