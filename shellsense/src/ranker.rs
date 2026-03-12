use crate::config::Config;
use crate::markov::MarkovPredictor;
use crate::fuzzy::FuzzyCorrector;
use crate::storage::Storage;
use crate::Suggestion;
use crate::SuggestionSource;
use std::collections::HashMap;

/// Combines all signal sources into a unified ranking
pub struct Ranker {
    markov: MarkovPredictor,
    fuzzy: FuzzyCorrector,
    config: Config,
}

impl Ranker {
    pub fn new(config: Config) -> Self {
        Ranker {
            markov: MarkovPredictor::new(),
            fuzzy: FuzzyCorrector::new(),
            config,
        }
    }

    /// Generate ranked suggestions combining all signals
    pub fn suggest(
        &self,
        storage: &Storage,
        prefix: Option<&str>,
        prev_cmd: Option<&str>,
        prev_cmd_2: Option<&str>,
        directory: Option<&str>,
        env: Option<&[String]>,
    ) -> Vec<Suggestion> {
        let limit = self.config.general.max_suggestions;
        let fetch_limit = limit * 3;
        let mut scored: HashMap<String, (f64, SuggestionSource)> = HashMap::new();
        let now = chrono::Utc::now().timestamp();

        // 1. Sequence predictions (Markov chain) — strongest signal
        if prev_cmd.is_some() {
            let predictions = self.markov.predict(storage, prev_cmd, prev_cmd_2, fetch_limit);
            for pred in predictions {
                let entry = scored.entry(pred.command.clone()).or_insert((0.0, SuggestionSource::Sequence));
                entry.0 += pred.score * self.config.weights.sequence;
            }
        }

        // 2. Prefix matches
        if let Some(pfx) = prefix {
            if !pfx.is_empty() {
                if let Ok(matches) = storage.get_prefix_matches(pfx, fetch_limit) {
                    let max_count = matches.first().map(|(_, c)| *c as f64).unwrap_or(1.0);
                    for (cmd, count) in matches {
                        let score = (count as f64) / max_count;
                        let entry = scored.entry(cmd).or_insert((0.0, SuggestionSource::Prefix));
                        entry.0 += score * self.config.weights.prefix;
                    }
                }
            }
        }

        // 3. Global frequency (only when no prefix to avoid double-counting)
        if prefix.is_none_or(|p| p.is_empty()) {
            if let Ok(freq) = storage.get_prefix_matches("", fetch_limit) {
                let max_count = freq.first().map(|(_, c)| *c as f64).unwrap_or(1.0);
                for (cmd, count) in freq {
                    let score = (count as f64) / max_count;
                    let entry = scored.entry(cmd).or_insert((0.0, SuggestionSource::Frequency));
                    entry.0 += score * self.config.weights.frequency;
                }
            }
        }

        // 4. Recency — batched query
        if !scored.is_empty() {
            let cmd_list: Vec<String> = scored.keys().cloned().collect();
            if let Ok(recency_data) = storage.get_batch_recency(&cmd_list) {
                for (cmd, last_used) in recency_data {
                    if let Some((ref mut total_score, _)) = scored.get_mut(&cmd) {
                        let age_hours = ((now - last_used) as f64) / 3600.0;
                        let recency_score = (-age_hours / 168.0).exp();
                        *total_score += recency_score * self.config.weights.recency;
                    }
                }
            }
        }

        // 5. Directory context
        if let Some(dir) = directory {
            if let Ok(dir_cmds) = storage.get_frequent_by_dir(dir, fetch_limit) {
                let max_count = dir_cmds.first().map(|(_, c)| *c as f64).unwrap_or(1.0);
                for (cmd, count) in dir_cmds {
                    let score = (count as f64) / max_count;
                    let entry = scored.entry(cmd).or_insert((0.0, SuggestionSource::Directory));
                    entry.0 += score * self.config.weights.directory;
                }
            }
        }

        // Apply contextual boosting based on active environment
        if let Some(envs) = env {
            for (cmd, (score, _)) in scored.iter_mut() {
                *score *= Self::apply_env_boost(cmd, envs);
            }
        }

        // Filter, sort, truncate
        let mut results: Vec<Suggestion> = scored
            .into_iter()
            .filter(|(cmd, (score, _))| {
                *score >= self.config.general.min_confidence
                && prefix.is_none_or(|p| p.is_empty() || cmd.starts_with(p))
                && !self.config.should_exclude(cmd)
            })
            .map(|(command, (score, source))| Suggestion { command, score, source })
            .collect();

        results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));

        // Apply path existence penalization only to the final top-N candidates (not all)
        if let Some(dir) = directory {
            for suggestion in results.iter_mut() {
                suggestion.score *= Self::penalize_invalid_context(&suggestion.command, dir);
            }
            // Re-sort after penalization
            results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        }

        results.truncate(limit);

        // Always try fuzzy correction when we have a prefix, even if we have some results.
        // This ensures typo corrections are available alongside normal suggestions.
        if let Some(pfx) = prefix {
            if !pfx.is_empty() && !results.iter().any(|s| s.source == SuggestionSource::Correction) {
                // Pre-filter candidates by first character similarity to reduce O(n) scan
                let first_char = pfx.chars().next();
                if let Ok(known) = storage.get_all_commands(1000) {
                    let filtered: Vec<String> = known.into_iter().filter(|k| {
                        let kc = k.chars().next();
                        // Keep if first char matches, or if first char is within QWERTY adjacency
                        match (first_char, kc) {
                            (Some(a), Some(b)) => {
                                a == b || Self::qwerty_adjacent(a, b) ||
                                // Also keep transposition candidates (2nd char match)
                                pfx.chars().nth(1) == k.chars().next()
                            }
                            _ => true,
                        }
                    }).collect();

                    if let Some(correction) = self.fuzzy.correct(pfx, &filtered) {
                        if !self.config.should_exclude(&correction.corrected)
                            && !results.iter().any(|s| s.command == correction.corrected)
                        {
                            // If we already have full results, replace the weakest one
                            if results.len() >= limit {
                                results.pop();
                            }
                            results.push(Suggestion {
                                command: correction.corrected,
                                score: correction.similarity,
                                source: SuggestionSource::Correction,
                            });
                        }
                    }
                }
            }
        }

        results
    }

    /// Check if two chars are adjacent on a QWERTY keyboard
    fn qwerty_adjacent(a: char, b: char) -> bool {
        let neighbors: &[(char, &[char])] = &[
            ('q', &['w', 'a']),
            ('w', &['q', 'e', 'a', 's']),
            ('e', &['w', 'r', 's', 'd']),
            ('r', &['e', 't', 'd', 'f']),
            ('t', &['r', 'y', 'f', 'g']),
            ('y', &['t', 'u', 'g', 'h']),
            ('u', &['y', 'i', 'h', 'j']),
            ('i', &['u', 'o', 'j', 'k']),
            ('o', &['i', 'p', 'k', 'l']),
            ('p', &['o', 'l']),
            ('a', &['q', 'w', 's', 'z']),
            ('s', &['w', 'e', 'a', 'd', 'z', 'x']),
            ('d', &['e', 'r', 's', 'f', 'x', 'c']),
            ('f', &['r', 't', 'd', 'g', 'c', 'v']),
            ('g', &['t', 'y', 'f', 'h', 'v', 'b']),
            ('h', &['y', 'u', 'g', 'j', 'b', 'n']),
            ('j', &['u', 'i', 'h', 'k', 'n', 'm']),
            ('k', &['i', 'o', 'j', 'l', 'm']),
            ('l', &['o', 'p', 'k']),
            ('z', &['a', 's', 'x']),
            ('x', &['s', 'd', 'z', 'c']),
            ('c', &['d', 'f', 'x', 'v']),
            ('v', &['f', 'g', 'c', 'b']),
            ('b', &['g', 'h', 'v', 'n']),
            ('n', &['h', 'j', 'b', 'm']),
            ('m', &['j', 'k', 'n']),
        ];

        let a_lower = a.to_ascii_lowercase();
        let b_lower = b.to_ascii_lowercase();

        for (ch, adj) in neighbors {
            if *ch == a_lower {
                return adj.contains(&b_lower);
            }
        }
        false
    }

    /// Boost commands based on active shell environment variables
    fn apply_env_boost(cmd: &str, envs: &[String]) -> f64 {
        let mut boost = 1.0;
        let base_cmd = cmd.split_whitespace().next().unwrap_or(cmd);

        for e in envs {
            let is_python = e == "VIRTUAL_ENV" && matches!(base_cmd, "python" | "pip" | "pytest" | "flask" | "django-admin");
            let is_k8s = e == "KUBECONFIG" && matches!(base_cmd, "kubectl" | "helm" | "k9s");

            if is_python || is_k8s {
                boost *= 1.5;
            } else if e == "AWS_PROFILE" && base_cmd == "aws" {
                boost *= 1.3;
            }
        }

        boost
    }

    /// Check if the command refers to a local file/directory that no longer exists
    /// Only called on finalized top-N candidates to minimize filesystem I/O
    fn penalize_invalid_context(cmd: &str, dir: &str) -> f64 {
        let parts: Vec<&str> = cmd.split_whitespace().collect();
        if parts.len() < 2 {
            return 1.0;
        }

        let base = parts[0];
        let target = parts.last().unwrap();

        // Check commands that typically point to local paths
        if matches!(base, "cd" | "cat" | "vim" | "nano" | "tail" | "less" | "code" | "rm" | "ls") {
            // Ignore flags, environment variables, and globs
            if target.starts_with('-') || target.contains('$') || target.contains('*') {
                return 1.0;
            }

            let resolved_target = if target.starts_with("~/") {
                if let Some(home) = dirs::home_dir() {
                    target.replacen("~/", &format!("{}/", home.display()), 1)
                } else {
                    target.to_string()
                }
            } else {
                target.to_string()
            };

            let path = std::path::Path::new(dir).join(resolved_target);
            if !path.exists() {
                return 0.1; // 90% penalty
            }
        }

        1.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::PrivacyConfig;

    fn setup() -> (Storage, Ranker) {
        let storage = Storage::open_memory().unwrap();
        let config = Config::default();
        let ranker = Ranker::new(config);
        (storage, ranker)
    }

    #[test]
    fn test_basic_suggestion() {
        let (storage, ranker) = setup();
        let now = chrono::Utc::now().timestamp();

        for _ in 0..5 {
            storage.add_command("git status", Some("/project"), None, Some(0), None, now, 10, None, None).unwrap();
            storage.add_command("git add .", Some("/project"), None, Some(0), None, now, 10, Some("git status"), None).unwrap();
        }

        let suggestions = ranker.suggest(&storage, Some("git"), None, None, Some("/project"), None);
        assert!(!suggestions.is_empty());
        assert!(suggestions.iter().any(|s| s.command.starts_with("git")));
    }

    #[test]
    fn test_sequence_suggestion() {
        let (storage, ranker) = setup();
        let now = chrono::Utc::now().timestamp();

        for _ in 0..10 {
            storage.add_command("git add .", Some("/p"), None, Some(0), None, now, 10, None, None).unwrap();
            storage.add_command("git commit -m 'x'", Some("/p"), None, Some(0), None, now, 10, Some("git add ."), None).unwrap();
        }

        let suggestions = ranker.suggest(&storage, None, Some("git add ."), None, Some("/p"), None);
        assert!(!suggestions.is_empty());
        assert_eq!(suggestions[0].command, "git commit -m 'x'");
    }

    #[test]
    fn test_fuzzy_fallback() {
        let (storage, ranker) = setup();
        let now = chrono::Utc::now().timestamp();

        for _ in 0..5 {
            storage.add_command("docker", None, None, Some(0), None, now, 10, None, None).unwrap();
        }

        let suggestions = ranker.suggest(&storage, Some("dokcer"), None, None, None, None);
        assert!(!suggestions.is_empty());
        assert_eq!(suggestions[0].command, "docker");
        assert_eq!(suggestions[0].source, SuggestionSource::Correction);
    }

    #[test]
    fn test_fuzzy_always_included() {
        // Even when there ARE prefix matches, a correction should still appear
        let (storage, ranker) = setup();
        let now = chrono::Utc::now().timestamp();

        // Add commands that start with "gti" AND "git"
        storage.add_command("git status", None, None, Some(0), None, now, 10, None, None).unwrap();
        storage.add_command("git add .", None, None, Some(0), None, now, 10, None, None).unwrap();
        // "gti" won't prefix-match "git", so fuzzy should fill in
        let suggestions = ranker.suggest(&storage, Some("gti"), None, None, None, None);
        assert!(suggestions.iter().any(|s| s.source == SuggestionSource::Correction));
    }

    #[test]
    fn test_empty_history() {
        let (storage, ranker) = setup();
        let suggestions = ranker.suggest(&storage, Some("git"), None, None, None, None);
        assert!(suggestions.is_empty());
    }

    #[test]
    fn test_privacy_filter_on_suggest() {
        let storage = Storage::open_memory().unwrap();
        let now = chrono::Utc::now().timestamp();

        storage.add_command("export PASSWORD=abc123", None, None, Some(0), None, now, 10, None, None).unwrap();
        storage.add_command("echo hello", None, None, Some(0), None, now, 10, None, None).unwrap();

        let config = Config {
            privacy: PrivacyConfig {
                exclude_patterns: vec!["*password*".to_string()],
            },
            ..Config::default()
        };
        let ranker = Ranker::new(config);

        let suggestions = ranker.suggest(&storage, Some("export"), None, None, None, None);
        assert!(suggestions.iter().all(|s| !s.command.contains("PASSWORD")));
    }

    #[test]
    fn test_no_duplicate_frequency_with_prefix() {
        let (storage, ranker) = setup();
        let now = chrono::Utc::now().timestamp();

        for _ in 0..5 {
            storage.add_command("ls -la", None, None, Some(0), None, now, 10, None, None).unwrap();
        }

        let suggestions = ranker.suggest(&storage, Some("ls"), None, None, None, None);
        assert!(!suggestions.is_empty());
        assert!(suggestions[0].score < 1.0);
    }

    #[test]
    fn test_apply_env_boost_python() {
        let envs = vec!["VIRTUAL_ENV".to_string()];
        let boost = Ranker::apply_env_boost("python -m pytest", &envs);
        assert!(boost > 1.0, "python command with VIRTUAL_ENV active should be boosted");

        let no_boost = Ranker::apply_env_boost("git status", &envs);
        assert!((no_boost - 1.0).abs() < f64::EPSILON, "non-python command should not be boosted");
    }

    #[test]
    fn test_apply_env_boost_k8s() {
        let envs = vec!["KUBECONFIG".to_string()];
        let boost = Ranker::apply_env_boost("kubectl get pods", &envs);
        assert!(boost > 1.0, "kubectl with KUBECONFIG should be boosted");
    }

    #[test]
    fn test_apply_env_boost_aws() {
        let envs = vec!["AWS_PROFILE".to_string()];
        let boost = Ranker::apply_env_boost("aws s3 ls", &envs);
        assert!(boost > 1.0, "aws with AWS_PROFILE should be boosted");

        let no_boost = Ranker::apply_env_boost("ls -la", &envs);
        assert!((no_boost - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_apply_env_boost_no_envs() {
        let envs: Vec<String> = vec![];
        let boost = Ranker::apply_env_boost("python test.py", &envs);
        assert!((boost - 1.0).abs() < f64::EPSILON, "no active envs should mean no boost");
    }

    #[test]
    fn test_penalize_invalid_context_no_args() {
        // Single-word commands have no path to check
        let penalty = Ranker::penalize_invalid_context("ls", "/tmp");
        assert!((penalty - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_penalize_invalid_context_flag() {
        // Flags should not be treated as paths
        let penalty = Ranker::penalize_invalid_context("ls -la", "/tmp");
        assert!((penalty - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_penalize_invalid_context_glob() {
        // Glob patterns should not be checked
        let penalty = Ranker::penalize_invalid_context("ls *.rs", "/tmp");
        assert!((penalty - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_penalize_invalid_context_nonexistent_path() {
        let penalty = Ranker::penalize_invalid_context("cd /definitely/does/not/exist/xyz123", "/tmp");
        assert!(penalty < 1.0, "nonexistent absolute path should be penalized");
    }

    #[test]
    fn test_qwerty_adjacent_true() {
        assert!(Ranker::qwerty_adjacent('q', 'w'));
        assert!(Ranker::qwerty_adjacent('s', 'd'));
        assert!(Ranker::qwerty_adjacent('f', 'g'));
    }

    #[test]
    fn test_qwerty_adjacent_false() {
        assert!(!Ranker::qwerty_adjacent('q', 'p'));
        assert!(!Ranker::qwerty_adjacent('a', 'l'));
        assert!(!Ranker::qwerty_adjacent('z', 'm'));
    }

    #[test]
    fn test_qwerty_adjacent_case_insensitive() {
        assert!(Ranker::qwerty_adjacent('Q', 'w'));
        assert!(Ranker::qwerty_adjacent('s', 'D'));
    }
}
