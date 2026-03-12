use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug, Deserialize, Clone)]
pub struct Config {
    #[serde(default = "default_general")]
    pub general: GeneralConfig,
    #[serde(default = "default_weights")]
    pub weights: WeightConfig,
    #[serde(default)]
    pub privacy: PrivacyConfig,
}

#[derive(Debug, Deserialize, Clone)]
pub struct GeneralConfig {
    #[serde(default = "default_max_suggestions")]
    pub max_suggestions: usize,
    #[serde(default = "default_min_confidence")]
    pub min_confidence: f64,
}

#[derive(Debug, Deserialize, Clone)]
pub struct WeightConfig {
    #[serde(default = "default_w_sequence")]
    pub sequence: f64,
    #[serde(default = "default_w_prefix")]
    pub prefix: f64,
    #[serde(default = "default_w_frequency")]
    pub frequency: f64,
    #[serde(default = "default_w_recency")]
    pub recency: f64,
    #[serde(default = "default_w_directory")]
    pub directory: f64,
}

#[derive(Debug, Deserialize, Default, Clone)]
pub struct PrivacyConfig {
    #[serde(default)]
    pub exclude_patterns: Vec<String>,
}

fn default_general() -> GeneralConfig {
    GeneralConfig {
        max_suggestions: default_max_suggestions(),
        min_confidence: default_min_confidence(),
    }
}

fn default_weights() -> WeightConfig {
    WeightConfig {
        sequence: default_w_sequence(),
        prefix: default_w_prefix(),
        frequency: default_w_frequency(),
        recency: default_w_recency(),
        directory: default_w_directory(),
    }
}

fn default_max_suggestions() -> usize { 5 }
fn default_min_confidence() -> f64 { 0.1 }
fn default_w_sequence() -> f64 { 0.40 }
fn default_w_prefix() -> f64 { 0.25 }
fn default_w_frequency() -> f64 { 0.15 }
fn default_w_recency() -> f64 { 0.10 }
fn default_w_directory() -> f64 { 0.10 }

impl Config {
    pub fn load() -> Self {
        let config_path = Self::config_path();
        if config_path.exists() {
            let content = std::fs::read_to_string(&config_path).unwrap_or_default();
            match toml::from_str(&content) {
                Ok(mut c) => {
                    let c: &mut Config = &mut c;
                    c.validate();
                    c.clone()
                }
                Err(e) => {
                    eprintln!("[shellsense warning] Failed to parse config.toml: {}", e);
                    eprintln!("[shellsense warning] Using default configuration.");
                    Self::default()
                }
            }
        } else {
            Self::default()
        }
    }

    /// Validate configuration values, clamping invalid weights
    fn validate(&mut self) {
        // Sanitize NaN/Inf weights to 0.0, then clamp negatives
        let sanitize = |v: f64| if v.is_finite() { v.max(0.0) } else { 0.0 };
        self.weights.sequence = sanitize(self.weights.sequence);
        self.weights.prefix = sanitize(self.weights.prefix);
        self.weights.frequency = sanitize(self.weights.frequency);
        self.weights.recency = sanitize(self.weights.recency);
        self.weights.directory = sanitize(self.weights.directory);

        // If all weights sum to effectively zero, restore defaults
        let sum = self.weights.sequence + self.weights.prefix + self.weights.frequency
            + self.weights.recency + self.weights.directory;
        if sum < f64::EPSILON {
            eprintln!("[shellsense warning] All weights are zero; using defaults.");
            self.weights = default_weights();
        }

        if self.general.max_suggestions == 0 {
            self.general.max_suggestions = 5;
        }

        self.general.min_confidence = self.general.min_confidence.clamp(0.0, 1.0);
    }

    pub fn data_dir() -> PathBuf {
        let home = match dirs::home_dir() {
            Some(h) => h,
            None => {
                eprintln!("[shellsense error] Cannot find home directory. Using /tmp/.shellsense as fallback.");
                PathBuf::from("/tmp")
            }
        };
        let dir = home.join(".shellsense");
        std::fs::create_dir_all(&dir).ok();
        dir
    }

    pub fn config_path() -> PathBuf {
        Self::data_dir().join("config.toml")
    }

    pub fn db_path() -> PathBuf {
        Self::data_dir().join("history.db")
    }

    /// Check if a command should be excluded based on privacy patterns.
    /// Patterns are treated as substring matches (glob `*` at edges is stripped).
    pub fn should_exclude(&self, command: &str) -> bool {
        let cmd_lower = command.to_lowercase();
        self.privacy.exclude_patterns.iter().any(|pattern| {
            let pat = pattern.trim_matches('*').to_lowercase();
            !pat.is_empty() && cmd_lower.contains(&pat)
        })
    }
}

impl Default for Config {
    fn default() -> Self {
        Config {
            general: default_general(),
            weights: default_weights(),
            privacy: PrivacyConfig::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = Config::default();
        assert_eq!(config.general.max_suggestions, 5);
        assert!((config.weights.sequence - 0.40).abs() < f64::EPSILON);
    }

    #[test]
    fn test_privacy_exclusion() {
        let config = Config {
            privacy: PrivacyConfig {
                exclude_patterns: vec!["*password*".to_string(), "*secret*".to_string()],
            },
            ..Config::default()
        };
        assert!(config.should_exclude("export PASSWORD=abc"));
        assert!(config.should_exclude("echo $SECRET_KEY"));
        assert!(!config.should_exclude("git commit -m 'hello'"));
    }

    #[test]
    fn test_empty_wildcard_pattern_does_not_match_everything() {
        let config = Config {
            privacy: PrivacyConfig {
                exclude_patterns: vec!["**".to_string(), "*".to_string(), "".to_string()],
            },
            ..Config::default()
        };
        // These should NOT be excluded — empty patterns after stripping must not match
        assert!(!config.should_exclude("git status"));
        assert!(!config.should_exclude("ls -la"));
        assert!(!config.should_exclude("echo hello"));
    }

    #[test]
    fn test_parse_config_toml() {
        let toml_str = r#"
[general]
max_suggestions = 10
min_confidence = 0.2

[weights]
sequence = 0.40
prefix = 0.25
frequency = 0.15
recency = 0.10
directory = 0.10

[privacy]
exclude_patterns = ["*token*"]
"#;
        let config: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(config.general.max_suggestions, 10);
        assert!((config.weights.sequence - 0.40).abs() < f64::EPSILON);
        assert_eq!(config.privacy.exclude_patterns, vec!["*token*"]);
    }

    #[test]
    fn test_validation_negative_weights() {
        let mut config = Config {
            weights: WeightConfig {
                sequence: -0.5,
                prefix: -0.1,
                frequency: 0.0,
                recency: 0.0,
                directory: 0.0,
            },
            ..Config::default()
        };
        config.validate();
        // All negative weights should be clamped to 0, then since sum is 0, defaults are restored
        assert!(config.weights.sequence > 0.0);
    }

    #[test]
    fn test_validation_zero_suggestions() {
        let mut config = Config {
            general: GeneralConfig {
                max_suggestions: 0,
                min_confidence: 2.0, // out of range
            },
            ..Config::default()
        };
        config.validate();
        assert_eq!(config.general.max_suggestions, 5);
        assert!(config.general.min_confidence <= 1.0);
    }

    #[test]
    fn test_validation_nan_inf_weights() {
        let mut config = Config {
            weights: WeightConfig {
                sequence: f64::NAN,
                prefix: f64::INFINITY,
                frequency: f64::NEG_INFINITY,
                recency: 0.0,
                directory: 0.0,
            },
            ..Config::default()
        };
        config.validate();
        // NaN/Inf should be sanitized then defaults restored since sum would be 0
        assert!(config.weights.sequence.is_finite());
        assert!(config.weights.prefix.is_finite());
        assert!(config.weights.frequency.is_finite());
        assert!(config.weights.sequence > 0.0, "defaults should be restored");
    }

    #[test]
    fn test_should_exclude_case_insensitive() {
        let config = Config {
            privacy: PrivacyConfig {
                exclude_patterns: vec!["*PASSWORD*".to_string()],
            },
            ..Config::default()
        };
        // Case-insensitive match
        assert!(config.should_exclude("export password=abc"));
        assert!(config.should_exclude("export PASSWORD=abc"));
        assert!(config.should_exclude("export Password=abc"));
    }

    #[test]
    fn test_should_exclude_unicode() {
        let config = Config {
            privacy: PrivacyConfig {
                exclude_patterns: vec!["*contraseña*".to_string()],
            },
            ..Config::default()
        };
        assert!(config.should_exclude("export CONTRASEÑA=secreto"));
        assert!(!config.should_exclude("echo hello"));
    }
}
