# M3U Unicode episode suffix regression

The synthetic HTTP playlist exposed a panic while parsing `Exemple / مثال S01 E02`.
The scanner returned a character index, which was then used as a byte offset to
slice the original UTF-8 title. The spawned parser task terminated and closed the
stream after seven of eight entries without an error item.

The focused `episode_suffix_uses_original_utf8_byte_offsets` regression test was
run before the correction and reproduced the panic. The scanner now retains
original byte offsets with `char_indices` and applies ASCII-only case folding to
the S/E markers, avoiding Unicode uppercase expansion such as `ß` to `SS`.
Arabic, accented French, German and emoji titles retain their original spelling.

All 14 M3U unit tests and crate Clippy pass after the correction. The HTTP fixture
also checks the complete eight-entry result, so early channel closure cannot pass
as a successful import. This changes no schema, profile or catalog identity rule.
Rollback is an isolated revert of the parser correction and its regression test.

This is local regression evidence, not full production import certification.
The [import validation guide](../reference/import-validation.md) requires:
“Never propose an import fix as done until the headless probe completes against a
real provider URL.” A successful real-provider probe and the same URL in the
running app remain outstanding. Do not merge on fixture coverage alone.
