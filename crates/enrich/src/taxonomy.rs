//! Zero-shot genre tagging. Embed the canonical genre labels once, then assign
//! each item the nearest label by cosine — but only when the best match clears a
//! confidence threshold, so weak guesses don't override the rule-based category.

use crate::cosine;

/// Holds the precomputed label embeddings (genre name → vector).
pub struct GenreClassifier {
    labels: Vec<(String, Vec<f32>)>,
}

impl GenreClassifier {
    pub fn new(labels: Vec<(String, Vec<f32>)>) -> Self {
        Self { labels }
    }

    /// Best genre for an item vector, or `None` if no label clears `threshold`.
    pub fn classify(&self, vector: &[f32], threshold: f32) -> Option<&str> {
        let mut best: Option<(&str, f32)> = None;
        for (name, label_vec) in &self.labels {
            let sim = cosine(vector, label_vec);
            if best.is_none_or(|(_, b)| sim > b) {
                best = Some((name.as_str(), sim));
            }
        }
        best.filter(|&(_, sim)| sim >= threshold)
            .map(|(name, _)| name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn classifier() -> GenreClassifier {
        GenreClassifier::new(vec![
            ("Action".to_string(), vec![1.0, 0.0, 0.0]),
            ("Comedy".to_string(), vec![0.0, 1.0, 0.0]),
            ("Drama".to_string(), vec![0.0, 0.0, 1.0]),
        ])
    }

    #[test]
    fn picks_nearest_label_above_threshold() {
        let c = classifier();
        assert_eq!(c.classify(&[0.9, 0.1, 0.0], 0.7), Some("Action"));
        assert_eq!(c.classify(&[0.0, 0.95, 0.05], 0.7), Some("Comedy"));
    }

    #[test]
    fn returns_none_when_below_threshold() {
        let c = classifier();
        // Equidistant / weak match stays unlabelled.
        assert_eq!(c.classify(&[0.58, 0.58, 0.58], 0.9), None);
    }
}
