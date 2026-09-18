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
