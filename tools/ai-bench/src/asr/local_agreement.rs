//! LocalAgreement-n streaming policy (Macháček et al. 2023, "Turning Whisper
//! into Real-Time Transcription System"): re-decode the audio buffer at every
//! step and commit the longest word prefix that the last two hypotheses agree
//! on. The rest is shown as provisional text. Pure logic, engine-agnostic.
//!
//! Each decode covers the whole buffer, so it repeats the committed words
//! whose audio is still buffered. Those are removed by aligning *text* (edit
//! distance), not timestamps: whisper's token timestamps are too rough at the
//! seam, and timestamp filtering both duplicated and dropped words.

use crate::text::normalize;

#[derive(Debug, Clone, PartialEq)]
pub struct Word {
    pub text: String,
    /// Absolute seconds in the stream.
    pub start: f64,
    pub end: f64,
}

#[derive(Debug, Default)]
pub struct LocalAgreement {
    committed: Vec<Word>,
    /// Previous hypothesis, after the committed words were removed.
    previous: Vec<Word>,
}

/// What a step produced: newly committed words and the current provisional tail.
#[derive(Debug, Default, PartialEq)]
pub struct StepOutput {
    pub committed: Vec<Word>,
    pub provisional: Vec<Word>,
}

fn key(w: &Word) -> String {
    normalize(&w.text)
}

/// Word-level edit distance between `a` and `b`.
fn distance(a: &[String], b: &[String]) -> usize {
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0; b.len() + 1];
    for (i, x) in a.iter().enumerate() {
        cur[0] = i + 1;
        for (j, y) in b.iter().enumerate() {
            cur[j + 1] = (prev[j] + usize::from(x != y))
                .min(prev[j + 1] + 1)
                .min(cur[j] + 1);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

/// How many leading words of `hypothesis` re-transcribe `committed_tail`.
/// Semi-global alignment: the tail may match after a few leading junk words
/// (after a trim, whisper often re-emits words from before the buffer) and
/// may end a few words early or late. The best match has the smallest edit
/// distance, then the *earliest* start (a later start would skip new words
/// that merely resemble the tail, e.g. tail "the" in "the man saw the dog"),
/// then the longest end (so a repeated word is consumed, not re-committed).
fn overlap(committed_tail: &[String], hypothesis: &[String]) -> usize {
    if committed_tail.is_empty() {
        return 0;
    }
    const LEADING_JUNK: usize = 6;
    // (distance, start, end)
    let mut best: Option<(usize, usize, usize)> = None;
    for start in 0..=LEADING_JUNK.min(hypothesis.len()) {
        let max_end = hypothesis.len().min(start + committed_tail.len() + 3);
        for end in start..=max_end {
            let d = distance(committed_tail, &hypothesis[start..end]);
            let better = match best {
                None => true,
                Some((bd, bs, be)) => {
                    d < bd || (d == bd && (start < bs || (start == bs && end > be)))
                }
            };
            if better {
                best = Some((d, start, end));
            }
        }
    }
    let (d, _, end) = best.unwrap_or((usize::MAX, 0, 0));
    // A poor match means the hypothesis does not cover the committed audio
    // (e.g. the buffer was trimmed past it): nothing to strip.
    if d * 2 > committed_tail.len() {
        0
    } else {
        end
    }
}

/// Cut a hypothesis where it degenerates into a loop, a known whisper failure
/// ("the present of the present of ..."): a 1–4 word phrase occurring at
/// least four times in a row and running to the end of the hypothesis.
/// Genuine repetition ("no no no", "très très très") is shorter or followed
/// by more speech, and is left alone.
pub fn truncate_repetition(words: &mut Vec<Word>) {
    let keys: Vec<String> = words.iter().map(key).collect();
    for i in 0..keys.len() {
        for n in 1..=4 {
            let repeats = (1..)
                .take_while(|r| {
                    let at = i + r * n;
                    at + n <= keys.len() && keys[at..at + n] == keys[i..i + n]
                })
                .count();
            let loop_end = i + (repeats + 1) * n;
            if repeats >= 3 && keys.len() - loop_end < n.max(2) {
                // Keep the first occurrence, drop the loop.
                words.truncate(i + n);
                return;
            }
        }
    }
}

impl LocalAgreement {
    /// End of the committed text.
    pub fn committed_until(&self) -> f64 {
        self.committed.last().map_or(0.0, |w| w.end)
    }

    /// Where the buffer may be trimmed: the end of the last committed word
    /// that closes a sentence (cutting mid-sentence loses words at the cut),
    /// if any is after `buffer_start`.
    pub fn trim_point(&self, buffer_start: f64) -> Option<f64> {
        self.committed
            .iter()
            .rev()
            .take_while(|w| w.end > buffer_start)
            .find(|w| w.text.trim_end().ends_with(['.', '?', '!', '؟', '…']))
            .map(|w| w.end)
    }

    pub fn committed(&self) -> &[Word] {
        &self.committed
    }

    /// Decoding prompt: the last few committed words whose audio is no
    /// longer in the buffer (before `buffer_start`). Text that is still in
    /// the buffer must not be prompted: the model would see it twice and skip
    /// or repeat it (whisper-streaming prompts the same way).
    pub fn prompt(&self, buffer_start: f64, max_words: usize) -> String {
        let before: Vec<&Word> = self
            .committed
            .iter()
            .filter(|w| w.end <= buffer_start + 0.05)
            .collect();
        let start = before.len().saturating_sub(max_words);
        before[start..]
            .iter()
            .map(|w| w.text.trim())
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// Feed one hypothesis covering the buffer that starts at `buffer_start`.
    pub fn step(&mut self, hypothesis: Vec<Word>, buffer_start: f64) -> StepOutput {
        // Committed words whose audio is still in the buffer are repeated at
        // the head of the hypothesis; strip them by text alignment.
        let tail: Vec<String> = self
            .committed
            .iter()
            .filter(|w| w.end > buffer_start + 0.05)
            .map(key)
            .collect();
        let keys: Vec<String> = hypothesis.iter().map(key).collect();
        let skip = overlap(&tail, &keys);
        let fresh: Vec<Word> = hypothesis.into_iter().skip(skip).collect();

        let agreed = fresh
            .iter()
            .zip(&self.previous)
            .take_while(|(a, b)| key(a) == key(b))
            .count();
        let committed: Vec<Word> = fresh[..agreed].to_vec();
        self.committed.extend(committed.iter().cloned());
        let provisional = fresh[agreed..].to_vec();
        self.previous = provisional.clone();
        StepOutput {
            committed,
            provisional,
        }
    }

    /// End of stream: whatever is still provisional is committed as is.
    pub fn flush(&mut self) -> Vec<Word> {
        let rest = std::mem::take(&mut self.previous);
        self.committed.extend(rest.iter().cloned());
        rest
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(spec: &[(&str, f64)]) -> Vec<Word> {
        spec.iter()
            .map(|(t, s)| Word {
                text: t.to_string(),
                start: *s,
                end: s + 0.4,
            })
            .collect()
    }

    fn texts(w: &[Word]) -> Vec<&str> {
        w.iter().map(|w| w.text.as_str()).collect()
    }

    #[test]
    fn commits_only_what_two_consecutive_hypotheses_agree_on() {
        let mut la = LocalAgreement::default();
        let first = la.step(words(&[("the", 0.0), ("cat", 0.5)]), 0.0);
        assert!(first.committed.is_empty(), "nothing to agree with yet");
        let second = la.step(words(&[("the", 0.0), ("cap", 0.5), ("sat", 1.0)]), 0.0);
        assert_eq!(texts(&second.committed), ["the"]);
        let third = la.step(
            words(&[("the", 0.0), ("cap", 0.5), ("sat", 1.0), ("down", 1.5)]),
            0.0,
        );
        assert_eq!(texts(&third.committed), ["cap", "sat"]);
        assert_eq!(la.flush().len(), 1);
        // Everything is still in a buffer starting at 0: nothing to prompt.
        assert_eq!(la.prompt(0.0, 10), "");
        assert_eq!(la.prompt(5.0, 10), "the cap sat down");
        assert_eq!(la.prompt(5.0, 2), "sat down");
    }

    #[test]
    fn agreement_ignores_case_and_punctuation() {
        let mut la = LocalAgreement::default();
        la.step(words(&[("Hello,", 0.0)]), 0.0);
        let out = la.step(words(&[("hello", 0.0), ("world", 0.5)]), 0.0);
        assert_eq!(out.committed.len(), 1);
    }

    /// Regression: with timestamp filtering, re-decoded committed words whose
    /// timestamps drifted were committed twice ("circulation est circulation est").
    #[test]
    fn repeated_committed_words_are_never_committed_again_despite_timestamp_drift() {
        let mut la = LocalAgreement::default();
        la.step(
            words(&[("la", 0.0), ("circulation", 0.4), ("est", 1.0)]),
            0.0,
        );
        la.step(
            words(&[("la", 0.0), ("circulation", 0.4), ("est", 1.0)]),
            0.0,
        );
        assert_eq!(texts(la.committed()), ["la", "circulation", "est"]);
        // Same buffer, timestamps shifted later by 0.6 s, plus new words.
        let drift = words(&[
            ("la", 0.6),
            ("circulation", 1.0),
            ("est", 1.6),
            ("dense", 2.0),
        ]);
        la.step(drift.clone(), 0.0);
        let out = la.step(drift, 0.0);
        assert_eq!(texts(&out.committed), ["dense"]);
        assert_eq!(texts(la.committed()), ["la", "circulation", "est", "dense"]);
    }

    /// Regression: with timestamp filtering, uncommitted words whose
    /// timestamps fell before the committed end were dropped ("on road").
    #[test]
    fn new_words_are_kept_even_when_their_timestamps_overlap_the_committed_end() {
        let mut la = LocalAgreement::default();
        la.step(words(&[("heavy", 0.0), ("on", 0.5)]), 0.0);
        la.step(words(&[("heavy", 0.0), ("on", 0.5)]), 0.0);
        // "the main" timestamped before "on" ended (rough token timestamps).
        let next = words(&[
            ("heavy", 0.0),
            ("on", 0.5),
            ("the", 0.6),
            ("main", 0.7),
            ("road", 1.2),
        ]);
        la.step(next.clone(), 0.0);
        la.step(next, 0.0);
        assert_eq!(
            texts(la.committed()),
            ["heavy", "on", "the", "main", "road"]
        );
    }

    #[test]
    fn after_a_trim_only_buffered_committed_words_are_matched() {
        let mut la = LocalAgreement::default();
        let a = words(&[("one.", 0.0), ("two", 1.0), ("three", 2.0)]);
        la.step(a.clone(), 0.0);
        la.step(a, 0.0);
        assert_eq!(la.trim_point(0.0), Some(0.4), "trim after the sentence end");
        // Buffer now starts at 0.4: the decode repeats "two three", then "four".
        let b = words(&[("two", 1.0), ("three", 2.0), ("four", 3.0)]);
        la.step(b.clone(), 0.4);
        la.step(b, 0.4);
        assert_eq!(texts(la.committed()), ["one.", "two", "three", "four"]);
    }

    /// Regression: after a trim the decode re-emitted "in the room." before
    /// the committed overlap, and those words were committed a second time.
    #[test]
    fn leading_junk_before_the_committed_overlap_is_not_recommitted() {
        let mut la = LocalAgreement::default();
        let first = words(&[
            ("in", 0.0),
            ("the", 0.3),
            ("room.", 0.6),
            ("as", 1.0),
            ("a", 1.2),
            ("result", 1.5),
        ]);
        la.step(first.clone(), 0.0);
        la.step(first, 0.0);
        // Buffer trimmed after "room." (1.0 s); the decode still starts with it.
        let next = words(&[
            ("in", 0.0),
            ("the", 0.3),
            ("room.", 0.6),
            ("as", 1.0),
            ("a", 1.2),
            ("result", 1.5),
            ("two", 2.0),
        ]);
        la.step(next.clone(), 1.0);
        la.step(next, 1.0);
        assert_eq!(
            texts(la.committed()),
            ["in", "the", "room.", "as", "a", "result", "two"]
        );
    }

    #[test]
    fn repetition_loops_are_cut_after_the_first_occurrence() {
        let mut w = words(&[
            ("ftir.", 0.0),
            ("the", 0.5),
            ("present", 1.0),
            ("of", 1.5),
            ("the", 2.0),
            ("present", 2.5),
            ("of", 3.0),
            ("the", 3.5),
            ("present", 4.0),
            ("of", 4.5),
            ("the", 5.0),
            ("present", 5.5),
            ("of", 6.0),
        ]);
        truncate_repetition(&mut w);
        assert_eq!(texts(&w), ["ftir.", "the", "present", "of"]);
    }

    /// Review: genuine repetition must survive ("no no no", "très très très").
    #[test]
    fn genuine_repetition_is_not_cut() {
        let mut short = words(&[("no", 0.0), ("no", 0.3), ("no", 0.6), ("stop", 1.0)]);
        truncate_repetition(&mut short);
        assert_eq!(short.len(), 4);
        let mut french = words(&[
            ("très", 0.0),
            ("très", 0.3),
            ("très", 0.6),
            ("très", 0.9),
            ("bien", 1.2),
            ("merci", 1.5),
            ("beaucoup", 1.8),
        ]);
        truncate_repetition(&mut french);
        assert_eq!(french.len(), 7, "followed by more speech: not a loop");
    }

    /// Review: on equal distance the earliest match wins, so new words that
    /// resemble a short committed tail are not skipped.
    #[test]
    fn a_short_tail_matches_its_first_occurrence_not_a_later_lookalike() {
        let tail = vec!["the".to_string()];
        let hyp: Vec<String> = ["the", "man", "saw", "the", "dog"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(overlap(&tail, &hyp), 1);
        let tail: Vec<String> = ["of", "the"].iter().map(|s| s.to_string()).collect();
        let hyp: Vec<String> = ["of", "the", "rest", "of", "the", "day"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(overlap(&tail, &hyp), 2);
    }
}
