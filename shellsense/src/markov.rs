use crate::storage::Storage;

/// Markov chain-based sequence predictor
/// Uses bigrams (order-1), trigrams (order-2), and base-command bigrams for generalization
pub struct MarkovPredictor {
    /// Weight for trigram predictions (higher = more trusted)
    trigram_weight: f64,
    /// Weight for bigram predictions
    bigram_weight: f64,
    /// Weight for base-command bigram predictions (generalization)
    base_bigram_weight: f64,
}

/// A prediction with its probability score
#[derive(Debug, Clone)]
pub struct Prediction {
    pub command: String,
    pub score: f64,
}

impl MarkovPredictor {
    pub fn new() -> Self {
        MarkovPredictor {
            trigram_weight: 0.50,
            bigram_weight: 0.35,
            base_bigram_weight: 0.15,
        }
    }

    /// Predict the next command given the last 1-2 commands
    /// Returns scored predictions sorted by probability (highest first)
    pub fn predict(
        &self,
        storage: &Storage,
        prev_cmd: Option<&str>,
        prev_cmd_2: Option<&str>,
        limit: usize,
    ) -> Vec<Prediction> {
        let mut predictions: std::collections::HashMap<String, f64> = std::collections::HashMap::new();

        // Trigram predictions (strongest signal — 3-command sequences)
        if let (Some(cmd2), Some(cmd1)) = (prev_cmd_2, prev_cmd) {
            if let Ok(trigrams) = storage.get_trigram_suggestions(cmd2, cmd1, limit * 2) {
                let total: f64 = trigrams.iter().map(|(_, c)| *c as f64).sum();
                if total > 0.0 {
                    for (cmd, count) in &trigrams {
                        let prob = (*count as f64) / total;
                        *predictions.entry(cmd.clone()).or_insert(0.0) += prob * self.trigram_weight;
                    }
                }
            }
        }

        // Bigram predictions (exact previous command)
        if let Some(cmd1) = prev_cmd {
            if let Ok(bigrams) = storage.get_bigram_suggestions(cmd1, limit * 2) {
                let total: f64 = bigrams.iter().map(|(_, c)| *c as f64).sum();
                if total > 0.0 {
                    for (cmd, count) in &bigrams {
                        let prob = (*count as f64) / total;
                        *predictions.entry(cmd.clone()).or_insert(0.0) += prob * self.bigram_weight;
                    }
                }
            }

            // Base-command bigrams (generalization: "after any git cmd → ...")
            if let Ok(base_bigrams) = storage.get_base_bigram_suggestions(cmd1, limit * 2) {
                let total: f64 = base_bigrams.iter().map(|(_, c)| *c as f64).sum();
                if total > 0.0 {
                    for (cmd, count) in &base_bigrams {
                        let prob = (*count as f64) / total;
                        *predictions.entry(cmd.clone()).or_insert(0.0) += prob * self.base_bigram_weight;
                    }
                }
            }
        }

        // Sort by score descending
        let mut result: Vec<Prediction> = predictions
            .into_iter()
            .map(|(command, score)| Prediction { command, score })
            .collect();
        result.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        result.truncate(limit);
        result
    }


}

impl Default for MarkovPredictor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup_storage() -> Storage {
        let storage = Storage::open_memory().unwrap();
        let now = chrono::Utc::now().timestamp();

        // Simulate a typical git workflow repeated multiple times
        for i in 0..10 {
            let ts = now + i;
            storage.add_command("git status", Some("/project"), None, Some(0), None, ts, 10, None, None).unwrap();
            storage.add_command("git add .", Some("/project"), None, Some(0), None, ts + 1, 10, Some("git status"), None).unwrap();
            storage.add_command("git commit -m 'update'", Some("/project"), None, Some(0), None, ts + 2, 10, Some("git add ."), Some("git status")).unwrap();
            storage.add_command("git push", Some("/project"), None, Some(0), None, ts + 3, 10, Some("git commit -m 'update'"), Some("git add .")).unwrap();
        }

        storage
    }

    #[test]
    fn test_bigram_prediction() {
        let storage = setup_storage();
        let predictor = MarkovPredictor::new();

        let preds = predictor.predict(&storage, Some("git status"), None, 5);
        assert!(!preds.is_empty());
        assert_eq!(preds[0].command, "git add .");
    }

    #[test]
    fn test_trigram_prediction() {
        let storage = setup_storage();
        let predictor = MarkovPredictor::new();

        let preds = predictor.predict(&storage, Some("git add ."), Some("git status"), 5);
        assert!(!preds.is_empty());
        assert_eq!(preds[0].command, "git commit -m 'update'");
    }

    #[test]
    fn test_no_history() {
        let storage = Storage::open_memory().unwrap();
        let predictor = MarkovPredictor::new();

        let preds = predictor.predict(&storage, Some("unknown_cmd"), None, 5);
        assert!(preds.is_empty());
    }

    #[test]
    fn test_base_command() {
        assert_eq!(crate::base_command("git commit -m 'test'"), "git");
        assert_eq!(crate::base_command("ls"), "ls");
        assert_eq!(crate::base_command(""), "");
    }

    #[test]
    fn test_base_bigram_generalization() {
        let storage = Storage::open_memory().unwrap();
        let now = chrono::Utc::now().timestamp();

        // Train: "git add ." → "git commit -m 'x'"
        for _ in 0..5 {
            storage.add_command("git add .", None, None, Some(0), None, now, 10, None, None).unwrap();
            storage.add_command("git commit -m 'x'", None, None, Some(0), None, now, 10, Some("git add ."), None).unwrap();
        }

        // Now predict after "git diff" — base command "git" should generalize
        let predictor = MarkovPredictor::new();
        let preds = predictor.predict(&storage, Some("git diff"), None, 5);
        // Should still suggest "git commit -m 'x'" via base_bigram path
        assert!(preds.iter().any(|p| p.command == "git commit -m 'x'"));
    }
}
