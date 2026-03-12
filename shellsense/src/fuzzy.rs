use strsim::normalized_levenshtein;

/// Fuzzy command corrector using string similarity
pub struct FuzzyCorrector {
    /// Maximum distance threshold (0.0 to 1.0, where 1.0 = exact match)
    min_similarity: f64,
}

#[derive(Debug, Clone)]
pub struct Correction {
    pub original: String,
    pub corrected: String,
    pub similarity: f64,
}

/// Calculate a primitive QWERTY physical distance penalty between two characters.
fn qwerty_distance(a: char, b: char) -> Option<f64> {
    let rows = [
        "qwertyuiop",
        "asdfghjkl",
        "zxcvbnm",
    ];
    
    let find_pos = |c: char| -> Option<(usize, usize)> {
        for (r_idx, row) in rows.iter().enumerate() {
            if let Some(c_idx) = row.find(c) {
                return Some((r_idx, c_idx));
            }
        }
        None
    };

    if let (Some(pos_a), Some(pos_b)) = (find_pos(a), find_pos(b)) {
        let r_diff = (pos_a.0 as f64 - pos_b.0 as f64).abs();
        let c_diff = (pos_a.1 as f64 - pos_b.1 as f64).abs();
        // Euclidean distance approximate
        Some((r_diff * r_diff + c_diff * c_diff).sqrt())
    } else {
        None
    }
}

impl FuzzyCorrector {
    pub fn new() -> Self {
        FuzzyCorrector {
            min_similarity: 0.6,
        }
    }

    /// Suggest a correction for a potentially mistyped command
    /// Compares against known commands and returns the best match
    pub fn suggest_correction(
        &self,
        input: &str,
        known_commands: &[String],
    ) -> Option<Correction> {
        if input.is_empty() || known_commands.is_empty() {
            return None;
        }

        // Extract base command (first word) for comparison
        let input_base = input.split_whitespace().next().unwrap_or(input);

        let mut best_match: Option<Correction> = None;
        let mut best_similarity = self.min_similarity;

        for known in known_commands {
            let known_base = known.split_whitespace().next().unwrap_or(known);

            // Compare base commands
            let mut sim = normalized_levenshtein(input_base, known_base);

            // Apply QWERTY adjacency booster if string lengths are similar
            if input_base.len() == known_base.len() {
                let mut dist_sum = 0.0;
                let mut matched_chars = 0;
                for (a, b) in input_base.chars().zip(known_base.chars()) {
                    if a != b {
                        if let Some(dist) = qwerty_distance(a, b) {
                            dist_sum += dist;
                            matched_chars += 1;
                        }
                    }
                }
                
                // If it's a single keystroke error and it's physically adjacent (dist <= 1.5)
                if matched_chars == 1 && dist_sum <= 1.5 {
                    sim += 0.2; // Massive boost for adjacent typos (e.g. 's' vs 'd')
                } else if matched_chars > 0 {
                    // Slight boost for other physical proximity
                    sim += 0.05 / dist_sum.clamp(1.0, 10.0);
                }
                sim = sim.clamp(0.0, 1.0);
            }

            if sim > best_similarity && sim < 1.0 {
                // Not exact match but close enough
                best_similarity = sim;

                // If input had arguments, try to preserve them with the corrected base
                let corrected = if input.contains(' ') && known_base != known {
                    // Both have args — suggest the full known command
                    known.clone()
                } else if input.contains(' ') {
                    // User typed args, known is just base — append user's args
                    let args = &input[input_base.len()..];
                    format!("{}{}", known_base, args)
                } else {
                    known_base.to_string()
                };

                best_match = Some(Correction {
                    original: input.to_string(),
                    corrected,
                    similarity: sim,
                });
            }
        }

        best_match
    }

    /// Check common transposition patterns (e.g., "gti" → "git")
    pub fn check_transposition(&self, input: &str, known_commands: &[String]) -> Option<Correction> {
        let input_base = input.split_whitespace().next().unwrap_or(input);
        let chars: Vec<char> = input_base.chars().collect();

        if chars.len() < 2 {
            return None;
        }

        use std::collections::HashSet;
        let mut known_bases = HashSet::new();
        for known in known_commands {
            known_bases.insert(known.split_whitespace().next().unwrap_or(known));
        }

        // Try all single-character transpositions
        for i in 0..chars.len() - 1 {
            let mut attempt = chars.clone();
            attempt.swap(i, i + 1);
            let transposed: String = attempt.into_iter().collect();

            if known_bases.contains(transposed.as_str()) {
                let known_base = transposed; // the transposition itself is the known base
                let corrected = if input.contains(' ') {
                    let args = &input[input_base.len()..];
                    format!("{}{}", known_base, args)
                } else {
                    known_base.to_string()
                };

                return Some(Correction {
                    original: input.to_string(),
                    corrected,
                    similarity: 0.95,
                });
            }
        }

        None
    }

    /// Combined correction: tries transposition first (fast), then fuzzy matching
    pub fn correct(
        &self,
        input: &str,
        known_commands: &[String],
    ) -> Option<Correction> {
        // First try transposition (very common typo pattern)
        if let Some(correction) = self.check_transposition(input, known_commands) {
            return Some(correction);
        }

        // Fall back to fuzzy matching
        self.suggest_correction(input, known_commands)
    }
}

impl Default for FuzzyCorrector {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn known() -> Vec<String> {
        vec![
            "git".to_string(),
            "git status".to_string(),
            "git commit".to_string(),
            "docker".to_string(),
            "docker-compose".to_string(),
            "npm".to_string(),
            "npm install".to_string(),
            "cargo".to_string(),
            "cargo build".to_string(),
            "kubectl".to_string(),
            "ls".to_string(),
            "cd".to_string(),
            "cat".to_string(),
        ]
    }

    #[test]
    fn test_transposition_gti() {
        let corrector = FuzzyCorrector::new();
        let result = corrector.correct("gti", &known());
        assert!(result.is_some());
        assert_eq!(result.unwrap().corrected, "git");
    }

    #[test]
    fn test_transposition_with_args() {
        let corrector = FuzzyCorrector::new();
        let result = corrector.correct("gti status", &known());
        assert!(result.is_some());
        assert_eq!(result.unwrap().corrected, "git status");
    }

    #[test]
    fn test_fuzzy_correction() {
        let corrector = FuzzyCorrector::new();
        let result = corrector.correct("dokcer", &known());
        assert!(result.is_some());
        assert_eq!(result.unwrap().corrected, "docker");
    }

    #[test]
    fn test_no_correction_needed() {
        let corrector = FuzzyCorrector::new();
        // Exact matches should not trigger correction
        let result = corrector.suggest_correction("git", &known());
        assert!(result.is_none());
    }

    #[test]
    fn test_too_different() {
        let corrector = FuzzyCorrector::new();
        let result = corrector.correct("xyzabc", &known());
        assert!(result.is_none());
    }

    #[test]
    fn test_empty_input() {
        let corrector = FuzzyCorrector::new();
        assert!(corrector.correct("", &known()).is_none());
        assert!(corrector.correct("git", &[]).is_none());
    }

    #[test]
    fn test_npm_typo() {
        let corrector = FuzzyCorrector::new();
        let result = corrector.correct("nmp install", &known());
        assert!(result.is_some());
        let c = result.unwrap();
        assert!(c.corrected.starts_with("npm"));
    }

    #[test]
    fn test_qwerty_distance_boost() {
        let corrector = FuzzyCorrector::new();
        // 's' and 'd' are adjacent on QWERTY
        let result_ls = corrector.correct("ld", &known());
        assert!(result_ls.is_some());
        assert_eq!(result_ls.unwrap().corrected, "ls");
    }

    #[test]
    fn test_fuzzy_unicode_goster() {
        let corrector = FuzzyCorrector::new();
        let known = vec!["göster log".to_string()];
        let result = corrector.correct("goster log", &known);
        assert!(result.is_some());
        assert_eq!(result.unwrap().corrected, "göster log");
    }
}
