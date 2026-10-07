//! Quality metrics: WER/CER for speech recognition, chrF++ and BLEU for
//! translation. chrF++ and BLEU reproduce sacreBLEU's corpus-level defaults
//! (chrF: char order 6, word order 2, beta 2; BLEU: 13a tokenizer, exp
//! smoothing) so numbers are comparable with published results; the tests
//! pin values computed with sacreBLEU itself.

use std::collections::HashMap;
use std::sync::LazyLock;

use regex::Regex;

use crate::text::normalize;

/// Edit distance between two token sequences.
fn levenshtein<T: PartialEq>(a: &[T], b: &[T]) -> usize {
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0; b.len() + 1];
    for (i, x) in a.iter().enumerate() {
        cur[0] = i + 1;
        for (j, y) in b.iter().enumerate() {
            let sub = prev[j] + usize::from(x != y);
            cur[j + 1] = sub.min(prev[j + 1] + 1).min(cur[j] + 1);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

/// Accumulated edit counts; corpus rates are total edits / total reference units.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct ErrorRate {
    pub edits: usize,
    pub reference: usize,
}

impl ErrorRate {
    pub fn rate(&self) -> f64 {
        if self.reference == 0 {
            0.0
        } else {
            self.edits as f64 / self.reference as f64
        }
    }

    pub fn add(&mut self, other: ErrorRate) {
        self.edits += other.edits;
        self.reference += other.reference;
    }
}

/// Word error counts after [`normalize`].
pub fn wer(reference: &str, hypothesis: &str) -> ErrorRate {
    let r = normalize(reference);
    let h = normalize(hypothesis);
    let r: Vec<&str> = r.split(' ').filter(|w| !w.is_empty()).collect();
    let h: Vec<&str> = h.split(' ').filter(|w| !w.is_empty()).collect();
    ErrorRate {
        edits: levenshtein(&r, &h),
        reference: r.len(),
    }
}

/// Character error counts after [`normalize`] (spaces included, as usual).
pub fn cer(reference: &str, hypothesis: &str) -> ErrorRate {
    let r: Vec<char> = normalize(reference).chars().collect();
    let h: Vec<char> = normalize(hypothesis).chars().collect();
    ErrorRate {
        edits: levenshtein(&r, &h),
        reference: r.len(),
    }
}

// ── chrF++ ──────────────────────────────────────────────────────────────────

const CHAR_ORDER: usize = 6;
const WORD_ORDER: usize = 2;
const BETA: f64 = 2.0;

fn counts<'a>(items: impl Iterator<Item = &'a str>) -> HashMap<&'a str, usize> {
    let mut map = HashMap::new();
    for item in items {
        *map.entry(item).or_insert(0) += 1;
    }
    map
}

/// sacreBLEU's chrF word split: a token's leading or trailing punctuation
/// character becomes its own token.
fn chrf_words(sentence: &str) -> Vec<String> {
    let mut words = Vec::new();
    for token in sentence.split_whitespace() {
        let chars: Vec<char> = token.chars().collect();
        if chars.len() > 1 && chars[chars.len() - 1].is_ascii_punctuation() {
            words.push(chars[..chars.len() - 1].iter().collect());
            words.push(chars[chars.len() - 1].to_string());
        } else if chars.len() > 1 && chars[0].is_ascii_punctuation() {
            words.push(chars[0].to_string());
            words.push(chars[1..].iter().collect());
        } else {
            words.push(token.to_string());
        }
    }
    words
}

/// Per-order (hyp, ref, match) statistics, char orders then word orders.
fn chrf_stats(hypothesis: &str, reference: &str) -> Vec<[usize; 3]> {
    let mut stats = Vec::with_capacity(CHAR_ORDER + WORD_ORDER);
    let squash = |s: &str| s.split_whitespace().collect::<String>();
    let (h, r) = (squash(hypothesis), squash(reference));
    let (hc, rc): (Vec<char>, Vec<char>) = (h.chars().collect(), r.chars().collect());
    for n in 1..=CHAR_ORDER {
        let grams =
            |c: &[char]| -> Vec<String> { c.windows(n).map(|w| w.iter().collect()).collect() };
        let (hg, rg) = (grams(&hc), grams(&rc));
        stats.push(chrf_counts(
            counts(hg.iter().map(String::as_str)),
            counts(rg.iter().map(String::as_str)),
        ));
    }
    let (hw, rw) = (chrf_words(hypothesis), chrf_words(reference));
    for n in 1..=WORD_ORDER {
        let grams = |w: &[String]| -> Vec<String> { w.windows(n).map(|g| g.join(" ")).collect() };
        let (hg, rg) = (grams(&hw), grams(&rw));
        stats.push(chrf_counts(
            counts(hg.iter().map(String::as_str)),
            counts(rg.iter().map(String::as_str)),
        ));
    }
    stats
}

fn match_counts(h: HashMap<&str, usize>, r: HashMap<&str, usize>) -> [usize; 3] {
    let matched = h
        .iter()
        .map(|(g, n)| (*n).min(*r.get(g).unwrap_or(&0)))
        .sum();
    [h.values().sum(), r.values().sum(), matched]
}

/// chrF statistics for one order: like [`match_counts`], but sacreBLEU does
/// not count hypothesis n-grams when the reference has none at that order.
fn chrf_counts(h: HashMap<&str, usize>, r: HashMap<&str, usize>) -> [usize; 3] {
    let [n_hyp, n_ref, matched] = match_counts(h, r);
    [if n_ref > 0 { n_hyp } else { 0 }, n_ref, matched]
}

/// Corpus chrF++ (0–100) over parallel hypothesis/reference lines.
pub fn chrf_pp(hypotheses: &[String], references: &[String]) -> f64 {
    let mut total = vec![[0usize; 3]; CHAR_ORDER + WORD_ORDER];
    for (h, r) in hypotheses.iter().zip(references) {
        for (t, s) in total.iter_mut().zip(chrf_stats(h, r)) {
            for k in 0..3 {
                t[k] += s[k];
            }
        }
    }
    // sacreBLEU's default (no eps smoothing): average precision and recall
    // over the orders that have both hypothesis and reference n-grams, then
    // take the F-beta of those averages.
    let factor = BETA * BETA;
    let (mut avg_prec, mut avg_rec, mut effective) = (0.0, 0.0, 0usize);
    for [n_hyp, n_ref, n_match] in total {
        if n_hyp > 0 && n_ref > 0 {
            avg_prec += n_match as f64 / n_hyp as f64;
            avg_rec += n_match as f64 / n_ref as f64;
            effective += 1;
        }
    }
    if effective == 0 {
        return 0.0;
    }
    avg_prec /= effective as f64;
    avg_rec /= effective as f64;
    if avg_prec + avg_rec == 0.0 {
        return 0.0;
    }
    100.0 * (1.0 + factor) * avg_prec * avg_rec / (factor * avg_prec + avg_rec)
}

// ── BLEU ────────────────────────────────────────────────────────────────────

static PERIOD_COMMA_A: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"([^0-9])([\.,])").unwrap());
static PERIOD_COMMA_B: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"([\.,])([^0-9])").unwrap());
static DIGIT_DASH: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"([0-9])(-)").unwrap());

fn is_13a_punct(c: char) -> bool {
    matches!(c, '{'..='~' | '['..='`' | ' '..='&' | '('..='+' | ':'..='@' | '/')
}

/// sacreBLEU's default "13a" tokenization.
fn tokenize_13a(line: &str) -> Vec<String> {
    let mut s = line
        .replace("<skipped>", "")
        .replace("-\n", "")
        .replace('\n', " ");
    if s.contains('&') {
        s = s
            .replace("&quot;", "\"")
            .replace("&amp;", "&")
            .replace("&lt;", "<")
            .replace("&gt;", ">");
    }
    let mut spaced = String::with_capacity(s.len() * 2);
    for c in format!(" {s} ").chars() {
        if is_13a_punct(c) {
            spaced.push(' ');
            spaced.push(c);
            spaced.push(' ');
        } else {
            spaced.push(c);
        }
    }
    let s = PERIOD_COMMA_A.replace_all(&spaced, "$1 $2 ");
    let s = PERIOD_COMMA_B.replace_all(&s, " $1 $2");
    let s = DIGIT_DASH.replace_all(&s, "$1 $2 ");
    s.split_whitespace().map(str::to_string).collect()
}

/// Corpus BLEU (0–100), single reference per line.
pub fn bleu(hypotheses: &[String], references: &[String]) -> f64 {
    const N: usize = 4;
    let (mut correct, mut totals) = ([0usize; N], [0usize; N]);
    let (mut sys_len, mut ref_len) = (0usize, 0usize);
    for (h, r) in hypotheses.iter().zip(references) {
        let (ht, rt) = (tokenize_13a(h), tokenize_13a(r));
        sys_len += ht.len();
        ref_len += rt.len();
        for n in 1..=N {
            let grams =
                |t: &[String]| -> Vec<String> { t.windows(n).map(|g| g.join(" ")).collect() };
            let (hg, rg) = (grams(&ht), grams(&rt));
            let [n_hyp, _, matched] = match_counts(
                counts(hg.iter().map(String::as_str)),
                counts(rg.iter().map(String::as_str)),
            );
            correct[n - 1] += matched;
            totals[n - 1] += n_hyp;
        }
    }
    if sys_len == 0 {
        return 0.0;
    }
    let mut log_sum = 0.0;
    let mut smooth = 1.0;
    for n in 0..N {
        if totals[n] == 0 {
            return 0.0;
        }
        let p = if correct[n] == 0 {
            smooth *= 2.0;
            100.0 / (smooth * totals[n] as f64)
        } else {
            100.0 * correct[n] as f64 / totals[n] as f64
        };
        log_sum += p.ln();
    }
    let bp = if sys_len >= ref_len {
        1.0
    } else {
        (1.0 - ref_len as f64 / sys_len as f64).exp()
    };
    bp * (log_sum / N as f64).exp()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(lines: &[&str]) -> Vec<String> {
        lines.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn wer_counts_substitutions_insertions_and_deletions() {
        let e = wer("the cat sat on the mat", "the cat sat on mat now");
        assert_eq!(
            e,
            ErrorRate {
                edits: 2,
                reference: 6
            }
        );
        assert_eq!(wer("Bonjour, le monde!", "bonjour le monde").edits, 0);
    }

    #[test]
    fn cer_counts_characters() {
        let e = cer("abc", "abd");
        assert_eq!(
            e,
            ErrorRate {
                edits: 1,
                reference: 3
            }
        );
    }

    #[test]
    fn identical_translations_score_100() {
        let lines = v(&["The cat is on the mat.", "Il fait beau aujourd'hui."]);
        assert!((chrf_pp(&lines, &lines) - 100.0).abs() < 1e-9);
        assert!((bleu(&lines, &lines) - 100.0).abs() < 1e-9);
    }

    #[test]
    fn tokenizer_13a_matches_sacrebleu() {
        assert_eq!(
            tokenize_13a("Hello, world! It's 3.5-4 (approx.)"),
            ["Hello", ",", "world", "!", "It's", "3.5", "-", "4", "(", "approx", ".", ")"]
        );
    }

    /// Reference values from sacreBLEU 2.5.1 (`corpus_chrf(word_order=2)`,
    /// `corpus_bleu` defaults) on the same English/French/Arabic lines.
    #[test]
    fn corpus_scores_match_sacrebleu() {
        let hyps = v(&[
            "The cat sat on the mat.",
            "Il fait très beau aujourd'hui, non ?",
            "ذهب الولد إلى المدرسة صباحا",
        ]);
        let refs = v(&[
            "The cat is sitting on the mat.",
            "Il fait beau aujourd'hui.",
            "ذهب الطفل إلى المدرسة في الصباح",
        ]);
        assert!((chrf_pp(&hyps, &refs) - 57.687255510725066).abs() < 1e-9);
        assert!((bleu(&hyps, &refs) - 24.281076515541944).abs() < 1e-9);
    }
}
