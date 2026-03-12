use crate::storage::Storage;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;

/// Import existing shell history into ShellSense
pub struct Importer;

impl Importer {
    /// Import zsh history file
    /// Supports both simple format and extended format with timestamps:
    ///   : 1234567890:0;command
    pub fn import_zsh_history(storage: &Storage) -> Result<(usize, usize), String> {
        let history_path = Self::find_zsh_history()
            .ok_or_else(|| "Could not find .zsh_history file".to_string())?;

        let file = std::fs::File::open(&history_path)
            .map_err(|e| format!("Failed to open history file: {}", e))?;
        let reader = BufReader::new(file);

        let mut total_cmds = 0;
        let mut total_patterns = 0;
        let mut batch = Vec::with_capacity(10_000);

        let mut multiline_buf = String::new();
        let mut multiline_ts: i64 = 0;

        let mut push_cmd = |cmd: String, ts: i64, batch: &mut Vec<(String, i64)>| {
            batch.push((cmd, ts));
            if batch.len() >= 10_000 {
                if let Ok((cmds, patterns)) = storage.bulk_add_commands(batch) {
                    total_cmds += cmds;
                    total_patterns += patterns;
                }
                batch.clear();
            }
        };

        for line_bytes in reader.split(b'\n') {
            let bytes = line_bytes.map_err(|e| format!("Read error: {}", e))?;
            let line = String::from_utf8_lossy(&bytes);

            // Handle multiline commands (ending with \)
            if !multiline_buf.is_empty() {
                multiline_buf.push('\n');
                if line.ends_with('\\') {
                    multiline_buf.push_str(line.trim_end_matches('\\'));
                } else {
                    multiline_buf.push_str(&line);
                    push_cmd(multiline_buf.clone(), multiline_ts, &mut batch);
                    multiline_buf.clear();
                }
                continue;
            }

            // Extended format: ": timestamp:duration;command"
            if line.starts_with(": ") {
                if let Some((ts, cmd)) = Self::parse_extended_line(&line) {
                    if cmd.ends_with('\\') {
                        multiline_buf = cmd.trim_end_matches('\\').to_string();
                        multiline_ts = ts;
                    } else if !cmd.is_empty() {
                        push_cmd(cmd, ts, &mut batch);
                    }
                }
            } else if !line.trim().is_empty() {
                // Simple format: just the command
                let now = chrono::Utc::now().timestamp();
                if line.ends_with('\\') {
                    multiline_buf = line.trim_end_matches('\\').to_string();
                    multiline_ts = now;
                } else {
                    push_cmd(line.to_string(), now, &mut batch);
                }
            }
        }

        if !multiline_buf.is_empty() {
            push_cmd(multiline_buf, multiline_ts, &mut batch);
        }

        if !batch.is_empty() {
            if let Ok((cmds, patterns)) = storage.bulk_add_commands(&batch) {
                total_cmds += cmds;
                total_patterns += patterns;
            }
        }

        Ok((total_cmds, total_patterns))
    }

    /// Parse history text into (command, timestamp) pairs
    /// Extracted for testability
    pub fn parse_history(text: &str) -> Vec<(String, i64)> {
        let mut commands: Vec<(String, i64)> = Vec::new();
        let mut multiline_buf = String::new();
        let mut multiline_ts: i64 = 0;

        for line in text.lines() {
            // Handle multiline commands (ending with \)
            if !multiline_buf.is_empty() {
                multiline_buf.push('\n');
                if line.ends_with('\\') {
                    multiline_buf.push_str(line.trim_end_matches('\\'));
                } else {
                    multiline_buf.push_str(line);
                    commands.push((multiline_buf.clone(), multiline_ts));
                    multiline_buf.clear();
                }
                continue;
            }

            // Extended format: ": timestamp:duration;command"
            if line.starts_with(": ") {
                if let Some((ts, cmd)) = Self::parse_extended_line(line) {
                    if cmd.ends_with('\\') {
                        multiline_buf = cmd.trim_end_matches('\\').to_string();
                        multiline_ts = ts;
                    } else if !cmd.is_empty() {
                        commands.push((cmd, ts));
                    }
                }
            } else if !line.trim().is_empty() {
                // Simple format: just the command
                let now = chrono::Utc::now().timestamp();
                if line.ends_with('\\') {
                    multiline_buf = line.trim_end_matches('\\').to_string();
                    multiline_ts = now;
                } else {
                    commands.push((line.to_string(), now));
                }
            }
        }

        // Flush any remaining multiline buffer
        if !multiline_buf.is_empty() {
            commands.push((multiline_buf, multiline_ts));
        }

        commands
    }

    /// Parse extended zsh history line format: ": timestamp:duration;command"
    /// Note: The command itself may contain semicolons, so we only split on the
    /// FIRST semicolon after the duration field
    fn parse_extended_line(line: &str) -> Option<(i64, String)> {
        // Strip ": " prefix
        let rest = line.strip_prefix(": ")?;

        // Find first ':' to split timestamp from the rest
        let colon_pos = rest.find(':')?;
        let timestamp: i64 = rest[..colon_pos].parse().ok()?;

        // Find the FIRST ';' after the colon — everything after it is the command
        let after_colon = &rest[colon_pos + 1..];
        let semicolon_pos = after_colon.find(';')?;
        let command = after_colon[semicolon_pos + 1..].to_string();

        Some((timestamp, command))
    }

    pub fn import_bash_history(storage: &Storage) -> Result<(usize, usize), String> {
        let history_path = Self::find_bash_history()
            .ok_or_else(|| "Could not find .bash_history file".to_string())?;

        let file = std::fs::File::open(&history_path)
            .map_err(|e| format!("Failed to open history file: {}", e))?;
        let reader = BufReader::new(file);
        
        // Count lines first to compute proper recency timestamps for bash
        let mut line_count = 0;
        for bytes in BufReader::new(std::fs::File::open(&history_path).unwrap()).split(b'\n').flatten() {
            let line = String::from_utf8_lossy(&bytes);
            if !line.trim().is_empty() && !line.starts_with('#') {
                line_count += 1;
            }
        }

        let mut total_cmds = 0;
        let mut total_patterns = 0;
        let mut batch = Vec::with_capacity(10_000);
        let now = chrono::Utc::now().timestamp();
        let mut parsed_count = 0;

        for line_bytes in reader.split(b'\n') {
            let bytes = line_bytes.map_err(|e| format!("Read error: {}", e))?;
            let line = String::from_utf8_lossy(&bytes);

            if !line.trim().is_empty() && !line.starts_with('#') {
                let doc_ts = now - (line_count as i64 - parsed_count as i64);
                parsed_count += 1;
                batch.push((line.to_string(), doc_ts));

                if batch.len() >= 10_000 {
                    if let Ok((cmds, patterns)) = storage.bulk_add_commands(&batch) {
                        total_cmds += cmds;
                        total_patterns += patterns;
                    }
                    batch.clear();
                }
            }
        }

        if !batch.is_empty() {
            if let Ok((cmds, patterns)) = storage.bulk_add_commands(&batch) {
                total_cmds += cmds;
                total_patterns += patterns;
            }
        }

        Ok((total_cmds, total_patterns))
    }

    /// Find the zsh history file
    fn find_zsh_history() -> Option<PathBuf> {
        // Check HISTFILE env var first
        if let Ok(histfile) = std::env::var("HISTFILE") {
            let path = PathBuf::from(histfile);
            if path.exists() {
                return Some(path);
            }
        }

        // Default locations
        if let Some(home) = dirs::home_dir() {
            let candidates = [
                home.join(".zsh_history"),
                home.join(".zhistory"),
                home.join(".histfile"),
            ];

            for path in &candidates {
                if path.exists() {
                    return Some(path.clone());
                }
            }
        }

        None
    }

    /// Find the bash history file
    fn find_bash_history() -> Option<PathBuf> {
        if let Some(home) = dirs::home_dir() {
            let path = home.join(".bash_history");
            if path.exists() {
                return Some(path);
            }
        }
        None
    }

    /// Import fish shell history
    /// Fish history uses a YAML-like format:
    ///   - cmd: some command
    ///     when: 1234567890
    pub fn import_fish_history(storage: &Storage) -> Result<(usize, usize), String> {
        let history_path = Self::find_fish_history()
            .ok_or_else(|| "Could not find fish history file".to_string())?;

        let content = std::fs::read_to_string(&history_path)
            .map_err(|e| format!("Failed to read fish history: {}", e))?;

        let mut batch = Vec::with_capacity(10_000);
        let mut total_cmds = 0;
        let mut total_patterns = 0;

        let mut current_cmd: Option<String> = None;
        let mut current_ts: i64 = 0;

        for line in content.lines() {
            if let Some(cmd) = line.strip_prefix("- cmd: ") {
                // Flush previous command
                if let Some(prev) = current_cmd.take() {
                    if !prev.trim().is_empty() {
                        batch.push((prev, current_ts));
                        if batch.len() >= 10_000 {
                            if let Ok((cmds, patterns)) = storage.bulk_add_commands(&batch) {
                                total_cmds += cmds;
                                total_patterns += patterns;
                            }
                            batch.clear();
                        }
                    }
                }
                current_cmd = Some(cmd.to_string());
                current_ts = chrono::Utc::now().timestamp(); // default if no `when:` follows
            } else if let Some(ts_str) = line.strip_prefix("  when: ") {
                if let Ok(ts) = ts_str.trim().parse::<i64>() {
                    current_ts = ts;
                }
            }
            // Ignore other lines (e.g. "  paths: ...")
        }

        // Flush last command
        if let Some(cmd) = current_cmd {
            if !cmd.trim().is_empty() {
                batch.push((cmd, current_ts));
            }
        }

        if !batch.is_empty() {
            if let Ok((cmds, patterns)) = storage.bulk_add_commands(&batch) {
                total_cmds += cmds;
                total_patterns += patterns;
            }
        }

        Ok((total_cmds, total_patterns))
    }

    /// Find the fish history file
    fn find_fish_history() -> Option<PathBuf> {
        // Check XDG data dir first
        if let Ok(xdg) = std::env::var("XDG_DATA_HOME") {
            let path = PathBuf::from(xdg).join("fish/fish_history");
            if path.exists() {
                return Some(path);
            }
        }

        if let Some(home) = dirs::home_dir() {
            let path = home.join(".local/share/fish/fish_history");
            if path.exists() {
                return Some(path);
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_extended_line() {
        let line = ": 1609459200:0;git status";
        let result = Importer::parse_extended_line(line);
        assert!(result.is_some());
        let (ts, cmd) = result.unwrap();
        assert_eq!(ts, 1609459200);
        assert_eq!(cmd, "git status");
    }

    #[test]
    fn test_parse_extended_with_duration() {
        let line = ": 1609459200:5;make build";
        let result = Importer::parse_extended_line(line);
        assert!(result.is_some());
        let (ts, cmd) = result.unwrap();
        assert_eq!(ts, 1609459200);
        assert_eq!(cmd, "make build");
    }

    #[test]
    fn test_parse_command_with_semicolons() {
        // Commands containing semicolons must preserve them
        let line = ": 1609459200:0;echo 'hello'; echo 'world'";
        let result = Importer::parse_extended_line(line);
        assert!(result.is_some());
        let (ts, cmd) = result.unwrap();
        assert_eq!(ts, 1609459200);
        assert_eq!(cmd, "echo 'hello'; echo 'world'");
    }

    #[test]
    fn test_parse_invalid_line() {
        assert!(Importer::parse_extended_line("not a valid line").is_none());
        assert!(Importer::parse_extended_line(": invalid:0;cmd").is_none());
    }

    #[test]
    fn test_parse_history_mixed_format() {
        let history = "\
: 1609459200:0;git status
: 1609459201:0;git add .
ls -la
pwd";
        let commands = Importer::parse_history(history);
        assert_eq!(commands.len(), 4);
        assert_eq!(commands[0].0, "git status");
        assert_eq!(commands[0].1, 1609459200);
        assert_eq!(commands[1].0, "git add .");
        assert_eq!(commands[2].0, "ls -la");
        assert_eq!(commands[3].0, "pwd");
    }

    #[test]
    fn test_parse_multiline_command() {
        let history = "\
: 1609459200:0;echo hello \\
world";
        let commands = Importer::parse_history(history);
        assert_eq!(commands.len(), 1);
        assert!(commands[0].0.contains("hello"));
        assert!(commands[0].0.contains("world"));
    }

    #[test]
    fn test_import_into_storage() {
        let storage = Storage::open_memory().unwrap();

        // Simulate bulk import
        let commands = vec![
            ("ls".to_string(), 1000),
            ("cd /tmp".to_string(), 1001),
            ("pwd".to_string(), 1002),
            ("ls -la".to_string(), 1003),
        ];

        let (total, bigrams) = storage.bulk_add_commands(&commands).unwrap();
        assert_eq!(total, 4);
        assert_eq!(bigrams, 3);

        // Check that bigrams were built
        let results = storage.get_bigram_suggestions("ls", 5).unwrap();
        assert!(!results.is_empty());
        assert_eq!(results[0].0, "cd /tmp");
    }

    #[test]
    fn test_parse_history_empty() {
        let commands = Importer::parse_history("");
        assert!(commands.is_empty());
    }

    #[test]
    fn test_parse_history_whitespace_only() {
        let commands = Importer::parse_history("   \n  \n\n");
        assert!(commands.is_empty(), "whitespace-only history lines should be ignored");
    }

    #[test]
    fn test_parse_history_single_command() {
        let commands = Importer::parse_history("ls");
        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0].0, "ls");
    }

    #[test]
    fn test_parse_history_unterminated_multiline() {
        // A multiline command that is never closed (file ends with backslash line)
        let history = ": 1609459200:0;echo hello \\";
        let commands = Importer::parse_history(history);
        assert_eq!(commands.len(), 1, "unterminated multiline should flush as-is");
        assert!(commands[0].0.contains("hello"));
    }
}
