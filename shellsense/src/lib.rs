pub mod storage;
pub mod markov;
pub mod fuzzy;
pub mod ranker;
pub mod config;
pub mod importer;
pub mod daemon;
pub mod tui;
pub mod shell;

/// Represents context about when/where a command was executed
#[derive(Debug, Clone)]
pub struct CommandContext {
    pub directory: Option<String>,
    pub git_branch: Option<String>,
    pub exit_code: Option<i32>,
    pub session_id: Option<String>,
    pub timestamp: i64,
    pub hour: u32,
}

/// A suggestion with its computed score
#[derive(Debug, Clone)]
pub struct Suggestion {
    pub command: String,
    pub score: f64,
    pub source: SuggestionSource,
}

/// Where a suggestion originated from
#[derive(Debug, Clone, PartialEq)]
pub enum SuggestionSource {
    Sequence,
    Prefix,
    Frequency,
    Directory,
    Correction,
}

impl std::fmt::Display for SuggestionSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SuggestionSource::Sequence => write!(f, "Sequence"),
            SuggestionSource::Prefix => write!(f, "Prefix"),
            SuggestionSource::Frequency => write!(f, "Frequency"),
            SuggestionSource::Directory => write!(f, "Directory"),
            SuggestionSource::Correction => write!(f, "Correction"),
        }
    }
}

impl std::fmt::Display for Suggestion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.command)
    }
}

/// Normalize command to its base form (first token) for broader pattern matching.
/// Shared utility to avoid duplication across modules.
pub fn base_command(cmd: &str) -> &str {
    cmd.split_whitespace().next().unwrap_or(cmd)
}
