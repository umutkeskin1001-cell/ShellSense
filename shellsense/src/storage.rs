use rusqlite::{Connection, params, Result};
use std::path::Path;
use crate::base_command;

const SCHEMA_VERSION: i32 = 2;

/// Persistent storage layer backed by SQLite
pub struct Storage {
    conn: Connection,
}

impl Storage {
    /// Open (or create) the database at the given path
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let conn = Connection::open(path)?;
        let storage = Storage { conn };
        storage.apply_pragmas()?;
        storage.init_tables()?;
        Ok(storage)
    }

    /// Create an in-memory database (for testing)
    #[cfg(test)]
    pub fn open_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        let storage = Storage { conn };
        storage.apply_pragmas()?;
        storage.init_tables()?;
        Ok(storage)
    }

    /// Apply SQLite performance pragmas
    fn apply_pragmas(&self) -> Result<()> {
        self.conn.execute_batch(
            "
            PRAGMA journal_mode = DELETE;
            PRAGMA synchronous = NORMAL;
            PRAGMA cache_size = -2000;
            PRAGMA temp_store = MEMORY;
            PRAGMA mmap_size = 268435456;
            PRAGMA busy_timeout = 3000;
            "
        )?;
        Ok(())
    }

    fn init_tables(&self) -> Result<()> {
        let version: i32 = self.conn.query_row("PRAGMA user_version;", [], |row| row.get(0)).unwrap_or(0);

        if version != SCHEMA_VERSION {
            self.recreate_schema()?;
        }
        Ok(())
    }

    fn recreate_schema(&self) -> Result<()> {
        self.conn.execute_batch(&format!(
            "
            DROP TABLE IF EXISTS command_history;
            DROP TABLE IF EXISTS bigrams;
            DROP TABLE IF EXISTS trigrams;
            DROP TABLE IF EXISTS command_freq;
            DROP TABLE IF EXISTS dir_freq;
            DROP TABLE IF EXISTS hour_freq;
            DROP TABLE IF EXISTS base_bigrams;

            CREATE TABLE command_history (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                command     TEXT NOT NULL,
                directory   TEXT,
                timestamp   INTEGER NOT NULL
            );

            CREATE INDEX idx_history_cmd ON command_history(command);
            CREATE INDEX idx_history_dir ON command_history(directory);
            CREATE INDEX idx_history_ts  ON command_history(timestamp);

            CREATE TABLE bigrams (
                prev_cmd    TEXT NOT NULL,
                next_cmd    TEXT NOT NULL,
                count       INTEGER DEFAULT 1,
                PRIMARY KEY (prev_cmd, next_cmd)
            );

            CREATE TABLE trigrams (
                cmd_2ago    TEXT NOT NULL,
                cmd_1ago    TEXT NOT NULL,
                next_cmd    TEXT NOT NULL,
                count       INTEGER DEFAULT 1,
                PRIMARY KEY (cmd_2ago, cmd_1ago, next_cmd)
            );

            CREATE TABLE command_freq (
                command     TEXT PRIMARY KEY,
                total_count INTEGER DEFAULT 1,
                last_used   INTEGER NOT NULL
            );

            CREATE TABLE dir_freq (
                directory   TEXT NOT NULL,
                command     TEXT NOT NULL,
                count       INTEGER DEFAULT 1,
                PRIMARY KEY (directory, command)
            );

            CREATE TABLE base_bigrams (
                prev_base   TEXT NOT NULL,
                next_cmd    TEXT NOT NULL,
                count       INTEGER DEFAULT 1,
                PRIMARY KEY (prev_base, next_cmd)
            );

            PRAGMA user_version = {};
            ",
            SCHEMA_VERSION
        ))?;
        Ok(())
    }



    /// Escape special LIKE characters in prefix to prevent injection
    fn escape_like(prefix: &str) -> String {
        prefix
            .replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_")
    }

    /// Record a command with its context, updating all frequency tables
    /// All writes are batched in a single transaction for performance
    #[allow(clippy::too_many_arguments)]
    pub fn add_command(
        &self,
        command: &str,
        directory: Option<&str>,
        _git_branch: Option<&str>,
        _exit_code: Option<i32>,
        _session_id: Option<&str>,
        timestamp: i64,
        _hour: u32,
        prev_cmd: Option<&str>,
        prev_cmd_2: Option<&str>,
    ) -> Result<()> {
        // Safety: unchecked_transaction is used because the daemon guarantees
        // single-writer access via its background writer thread.
        let tx = self.conn.unchecked_transaction()?;

        // Insert into history
        tx.execute(
            "INSERT INTO command_history (command, directory, timestamp)
             VALUES (?1, ?2, ?3)",
            params![command, directory, timestamp],
        )?;

        // Update command frequency
        tx.execute(
            "INSERT INTO command_freq (command, total_count, last_used)
             VALUES (?1, 1, ?2)
             ON CONFLICT(command) DO UPDATE SET
                total_count = total_count + 1,
                last_used = MAX(last_used, ?2)",
            params![command, timestamp],
        )?;

        // Update directory frequency
        if let Some(dir) = directory {
            tx.execute(
                "INSERT INTO dir_freq (directory, command, count)
                 VALUES (?1, ?2, 1)
                 ON CONFLICT(directory, command) DO UPDATE SET count = count + 1",
                params![dir, command],
            )?;
        }

        // Update bigrams
        if let Some(prev) = prev_cmd {
            tx.execute(
                "INSERT INTO bigrams (prev_cmd, next_cmd, count)
                 VALUES (?1, ?2, 1)
                 ON CONFLICT(prev_cmd, next_cmd) DO UPDATE SET count = count + 1",
                params![prev, command],
            )?;

            // Also update base-command bigrams for generalization
            let prev_base = base_command(prev);
            tx.execute(
                "INSERT INTO base_bigrams (prev_base, next_cmd, count)
                 VALUES (?1, ?2, 1)
                 ON CONFLICT(prev_base, next_cmd) DO UPDATE SET count = count + 1",
                params![prev_base, command],
            )?;
        }

        // Update trigrams
        if let Some(prev2) = prev_cmd_2 {
            if let Some(prev1) = prev_cmd {
                tx.execute(
                    "INSERT INTO trigrams (cmd_2ago, cmd_1ago, next_cmd, count)
                     VALUES (?1, ?2, ?3, 1)
                     ON CONFLICT(cmd_2ago, cmd_1ago, next_cmd) DO UPDATE SET count = count + 1",
                    params![prev2, prev1, command],
                )?;
            }
        }

        tx.commit()?;
        Ok(())
    }

    /// Get bigram suggestions: commands that typically follow `prev_cmd`
    pub fn get_bigram_suggestions(&self, prev_cmd: &str, limit: usize) -> Result<Vec<(String, u32)>> {
        let mut stmt = self.conn.prepare(
            "SELECT next_cmd, count FROM bigrams
             WHERE prev_cmd = ?1
             ORDER BY count DESC
             LIMIT ?2"
        )?;
        let rows = stmt.query_map(params![prev_cmd, limit as u32], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, u32>(1)?))
        })?;
        rows.collect()
    }

    /// Get base-command bigram suggestions for broader matching
    /// e.g., if prev was "git add .", this matches any command starting with "git"
    pub fn get_base_bigram_suggestions(&self, prev_cmd: &str, limit: usize) -> Result<Vec<(String, u32)>> {
        let base = crate::base_command(prev_cmd);
        let mut stmt = self.conn.prepare(
            "SELECT next_cmd, count FROM base_bigrams
             WHERE prev_base = ?1
             ORDER BY count DESC
             LIMIT ?2"
        )?;
        let rows = stmt.query_map(params![base, limit as u32], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, u32>(1)?))
        })?;
        rows.collect()
    }

    /// Get trigram suggestions: commands that typically follow (cmd_2, cmd_1) sequence
    pub fn get_trigram_suggestions(&self, cmd_2: &str, cmd_1: &str, limit: usize) -> Result<Vec<(String, u32)>> {
        let mut stmt = self.conn.prepare(
            "SELECT next_cmd, count FROM trigrams
             WHERE cmd_2ago = ?1 AND cmd_1ago = ?2
             ORDER BY count DESC
             LIMIT ?3"
        )?;
        let rows = stmt.query_map(params![cmd_2, cmd_1, limit as u32], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, u32>(1)?))
        })?;
        rows.collect()
    }

    /// Get commands matching a prefix, ordered by frequency
    /// Uses proper LIKE escaping to handle special characters
    pub fn get_prefix_matches(&self, prefix: &str, limit: usize) -> Result<Vec<(String, u32)>> {
        let pattern = format!("{}%", Self::escape_like(prefix));
        let mut stmt = self.conn.prepare(
            "SELECT command, total_count FROM command_freq
             WHERE command LIKE ?1 ESCAPE '\\'
             ORDER BY total_count DESC
             LIMIT ?2"
        )?;
        let rows = stmt.query_map(params![pattern, limit as u32], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, u32>(1)?))
        })?;
        rows.collect()
    }

    /// Get most frequent commands in a specific directory
    pub fn get_frequent_by_dir(&self, directory: &str, limit: usize) -> Result<Vec<(String, u32)>> {
        let mut stmt = self.conn.prepare(
            "SELECT command, count FROM dir_freq
             WHERE directory = ?1
             ORDER BY count DESC
             LIMIT ?2"
        )?;
        let rows = stmt.query_map(params![directory, limit as u32], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, u32>(1)?))
        })?;
        rows.collect()
    }

    /// Get all known commands (for fuzzy matching)
    pub fn get_all_commands(&self, limit: usize) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT command FROM command_freq ORDER BY total_count DESC LIMIT ?1"
        )?;
        let rows = stmt.query_map(params![limit as u32], |row| {
            row.get::<_, String>(0)
        })?;
        rows.collect()
    }

    /// Get top N most frequent commands with their counts (for stats display)
    pub fn get_top_commands(&self, limit: usize) -> Result<Vec<(String, u32)>> {
        let mut stmt = self.conn.prepare(
            "SELECT command, total_count FROM command_freq ORDER BY total_count DESC LIMIT ?1"
        )?;
        let rows = stmt.query_map(params![limit as u32], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, u32>(1)?))
        })?;
        rows.collect()
    }

    /// Batch get recency data for multiple commands in a single query
    /// Returns (command, last_used) pairs — avoids N+1 query pattern
    /// Automatically batches queries to respect SQLite's parameter limit (999)
    pub fn get_batch_recency(&self, commands: &[String]) -> Result<Vec<(String, i64)>> {
        if commands.is_empty() {
            return Ok(Vec::new());
        }

        const SQLITE_MAX_PARAMS: usize = 999;
        let mut all_results = Vec::new();

        for chunk in commands.chunks(SQLITE_MAX_PARAMS) {
            let placeholders: Vec<String> = (1..=chunk.len())
                .map(|i| format!("?{}", i))
                .collect();
            let sql = format!(
                "SELECT command, last_used FROM command_freq WHERE command IN ({})",
                placeholders.join(",")
            );

            let mut stmt = self.conn.prepare(&sql)?;
            let params: Vec<&dyn rusqlite::types::ToSql> = chunk
                .iter()
                .map(|s| s as &dyn rusqlite::types::ToSql)
                .collect();

            let rows = stmt.query_map(params.as_slice(), |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })?;

            for row in rows {
                all_results.push(row?);
            }
        }

        Ok(all_results)
    }

    /// Get total command count (for statistics)
    pub fn total_commands(&self) -> Result<u64> {
        self.conn.query_row(
            "SELECT COUNT(*) FROM command_history",
            [],
            |row| row.get(0),
        )
    }

    /// Get unique command count
    pub fn unique_commands(&self) -> Result<u64> {
        self.conn.query_row(
            "SELECT COUNT(*) FROM command_freq",
            [],
            |row| row.get(0),
        )
    }

    /// Get total pattern count (bigrams + trigrams)
    pub fn total_patterns(&self) -> Result<(u64, u64)> {
        let bigrams: u64 = self.conn.query_row(
            "SELECT COUNT(*) FROM bigrams", [], |row| row.get(0)
        )?;
        let trigrams: u64 = self.conn.query_row(
            "SELECT COUNT(*) FROM trigrams", [], |row| row.get(0)
        )?;
        Ok((bigrams, trigrams))
    }

    /// Get database file size in bytes
    pub fn db_size_bytes(&self) -> Result<i64> {
        self.conn.query_row(
            "SELECT page_count * page_size FROM pragma_page_count(), pragma_page_size()",
            [],
            |row| row.get(0),
        )
    }

    /// Reset the entire database (delete all learned data)
    pub fn reset(&self) -> Result<()> {
        self.conn.execute_batch(
            "
            DELETE FROM command_history;
            DELETE FROM bigrams;
            DELETE FROM trigrams;
            DELETE FROM base_bigrams;
            DELETE FROM command_freq;
            DELETE FROM dir_freq;
            VACUUM;
            "
        )?;
        Ok(())
    }

    /// Prune old command history and reclaim disk space
    /// Aggregated analytics (frequencies, bigrams) are KEPT. Only raw history string logs are deleted.
    pub fn vacuum_and_prune(&self, retain_days: i64) -> Result<()> {
        let cutoff_seconds = chrono::Utc::now().timestamp() - (retain_days * 24 * 60 * 60);

        self.conn.execute(
            "DELETE FROM command_history WHERE timestamp < ?1",
            params![cutoff_seconds],
        )?;

        self.conn.execute_batch("VACUUM;")?;
        Ok(())
    }

    /// Delete a specific command completely from history and all frequency tables
    pub fn delete_command(&self, cmd: &str) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;

        tx.execute("DELETE FROM command_history WHERE command = ?1", params![cmd])?;
        tx.execute("DELETE FROM command_freq WHERE command = ?1", params![cmd])?;
        tx.execute("DELETE FROM dir_freq WHERE command = ?1", params![cmd])?;
        tx.execute("DELETE FROM bigrams WHERE prev_cmd = ?1 OR next_cmd = ?1", params![cmd])?;
        // Only delete base_bigrams where this command appears as next_cmd.
        // Do NOT delete by prev_base — that would wipe unrelated entries sharing the same
        // base command (e.g. deleting "git status" would destroy all "git" → * patterns).
        tx.execute("DELETE FROM base_bigrams WHERE next_cmd = ?1", params![cmd])?;
        tx.execute("DELETE FROM trigrams WHERE cmd_2ago = ?1 OR cmd_1ago = ?1 OR next_cmd = ?1", params![cmd])?;


        tx.commit()?;
        Ok(())
    }

    /// Bulk insert for history import (uses transaction for speed)
    pub fn bulk_add_commands(&self, commands: &[(String, i64)]) -> Result<(usize, usize)> {
        // Safety: bulk import runs single-threaded before daemon starts
        let tx = self.conn.unchecked_transaction()?;
        let mut bigram_count = 0usize;
        let mut prev_cmd: Option<String> = None;
        let mut prev_cmd_2: Option<String> = None;

        for (cmd, ts) in commands {
            // Insert into history
            tx.execute(
                "INSERT INTO command_history (command, directory, timestamp)
                 VALUES (?1, NULL, ?2)",
                params![cmd, ts],
            )?;

            // Update command frequency
            tx.execute(
                "INSERT INTO command_freq (command, total_count, last_used)
                 VALUES (?1, 1, ?2)
                 ON CONFLICT(command) DO UPDATE SET
                    total_count = total_count + 1,
                    last_used = MAX(last_used, ?2)",
                params![cmd, ts],
            )?;

            // Update bigrams
            if let Some(ref prev) = prev_cmd {
                tx.execute(
                    "INSERT INTO bigrams (prev_cmd, next_cmd, count)
                     VALUES (?1, ?2, 1)
                     ON CONFLICT(prev_cmd, next_cmd) DO UPDATE SET count = count + 1",
                    params![prev, cmd],
                )?;
                bigram_count += 1;

                // Base-command bigrams for generalization
                let prev_base = base_command(prev);
                tx.execute(
                    "INSERT INTO base_bigrams (prev_base, next_cmd, count)
                     VALUES (?1, ?2, 1)
                     ON CONFLICT(prev_base, next_cmd) DO UPDATE SET count = count + 1",
                    params![prev_base, cmd],
                )?;
            }

            // Update trigrams
            if let Some(ref p2) = prev_cmd_2 {
                if let Some(ref p1) = prev_cmd {
                    tx.execute(
                        "INSERT INTO trigrams (cmd_2ago, cmd_1ago, next_cmd, count)
                         VALUES (?1, ?2, ?3, 1)
                         ON CONFLICT(cmd_2ago, cmd_1ago, next_cmd) DO UPDATE SET count = count + 1",
                        params![p2, p1, cmd],
                    )?;
                }
            }

            prev_cmd_2 = prev_cmd.take();
            prev_cmd = Some(cmd.clone());
        }

        tx.commit()?;
        Ok((commands.len(), bigram_count))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::OptionalExtension;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_db_path(label: &str) -> PathBuf {
        let mut path = std::env::temp_dir();
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before unix epoch")
            .as_nanos();
        path.push(format!(
            "shellsense-storage-{label}-{}-{nanos}",
            std::process::id()
        ));
        fs::create_dir_all(&path).unwrap();
        path.join("history.db")
    }

    #[test]
    fn test_add_and_query_command() {
        let storage = Storage::open_memory().unwrap();
        let now = chrono::Utc::now().timestamp();

        storage.add_command("git status", Some("/home/user/project"), None, Some(0), None, now, 10, None, None).unwrap();
        storage.add_command("git add .", Some("/home/user/project"), None, Some(0), None, now, 10, Some("git status"), None).unwrap();
        storage.add_command("git commit -m 'test'", Some("/home/user/project"), None, Some(0), None, now, 10, Some("git add ."), Some("git status")).unwrap();

        // Test prefix matching
        let results = storage.get_prefix_matches("git", 10).unwrap();
        assert_eq!(results.len(), 3);

        // Test bigram
        let bigrams = storage.get_bigram_suggestions("git status", 10).unwrap();
        assert_eq!(bigrams.len(), 1);
        assert_eq!(bigrams[0].0, "git add .");

        // Test trigram
        let trigrams = storage.get_trigram_suggestions("git status", "git add .", 10).unwrap();
        assert_eq!(trigrams.len(), 1);
        assert_eq!(trigrams[0].0, "git commit -m 'test'");
    }

    #[test]
    fn test_frequency_counting() {
        let storage = Storage::open_memory().unwrap();
        let now = chrono::Utc::now().timestamp();

        for _ in 0..5 {
            storage.add_command("ls", Some("/home"), None, Some(0), None, now, 10, None, None).unwrap();
        }
        for _ in 0..3 {
            storage.add_command("pwd", Some("/home"), None, Some(0), None, now, 10, None, None).unwrap();
        }

        let results = storage.get_prefix_matches("", 10).unwrap();
        assert_eq!(results[0].0, "ls");
        assert_eq!(results[0].1, 5);
        assert_eq!(results[1].0, "pwd");
        assert_eq!(results[1].1, 3);
    }

    #[test]
    fn test_directory_frequency() {
        let storage = Storage::open_memory().unwrap();
        let now = chrono::Utc::now().timestamp();

        storage.add_command("npm test", Some("/project"), None, Some(0), None, now, 10, None, None).unwrap();
        storage.add_command("npm test", Some("/project"), None, Some(0), None, now, 10, None, None).unwrap();
        storage.add_command("cargo test", Some("/rust-project"), None, Some(0), None, now, 10, None, None).unwrap();

        let results = storage.get_frequent_by_dir("/project", 10).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, "npm test");
        assert_eq!(results[0].1, 2);
    }

    #[test]
    fn test_bulk_import() {
        let storage = Storage::open_memory().unwrap();
        let now = chrono::Utc::now().timestamp();

        let commands = vec![
            ("ls".to_string(), now - 100),
            ("cd /tmp".to_string(), now - 90),
            ("ls -la".to_string(), now - 80),
        ];

        let (total, bigrams) = storage.bulk_add_commands(&commands).unwrap();
        assert_eq!(total, 3);
        assert_eq!(bigrams, 2);

        let results = storage.get_bigram_suggestions("ls", 10).unwrap();
        assert_eq!(results[0].0, "cd /tmp");
    }

    #[test]
    fn test_stats() {
        let storage = Storage::open_memory().unwrap();
        let now = chrono::Utc::now().timestamp();

        storage.add_command("ls", None, None, None, None, now, 10, None, None).unwrap();
        storage.add_command("pwd", None, None, None, None, now, 10, Some("ls"), None).unwrap();

        assert_eq!(storage.total_commands().unwrap(), 2);
        assert_eq!(storage.unique_commands().unwrap(), 2);
        let (bi, _tri) = storage.total_patterns().unwrap();
        assert_eq!(bi, 1);
    }

    #[test]
    fn test_reset() {
        let storage = Storage::open_memory().unwrap();
        let now = chrono::Utc::now().timestamp();

        storage.add_command("ls", None, None, None, None, now, 10, None, None).unwrap();
        storage.add_command("pwd", None, None, None, None, now, 10, Some("ls"), None).unwrap();

        assert_eq!(storage.total_commands().unwrap(), 2);
        storage.reset().unwrap();
        assert_eq!(storage.total_commands().unwrap(), 0);
        assert_eq!(storage.unique_commands().unwrap(), 0);
    }

    #[test]
    fn test_batch_recency() {
        let storage = Storage::open_memory().unwrap();
        let now = chrono::Utc::now().timestamp();

        storage.add_command("ls", None, None, None, None, now - 100, 10, None, None).unwrap();
        storage.add_command("pwd", None, None, None, None, now, 10, None, None).unwrap();

        let cmds = vec!["ls".to_string(), "pwd".to_string(), "nonexistent".to_string()];
        let results = storage.get_batch_recency(&cmds).unwrap();
        assert_eq!(results.len(), 2); // nonexistent excluded
    }

    #[test]
    fn test_base_bigrams() {
        let storage = Storage::open_memory().unwrap();
        let now = chrono::Utc::now().timestamp();

        storage.add_command("git add .", None, None, Some(0), None, now, 10, None, None).unwrap();
        storage.add_command("git commit -m 'x'", None, None, Some(0), None, now, 10, Some("git add ."), None).unwrap();

        // Base bigrams: "git" → "git commit -m 'x'"
        let results = storage.get_base_bigram_suggestions("git status", 10).unwrap();
        assert!(!results.is_empty());
        assert_eq!(results[0].0, "git commit -m 'x'");
    }

    #[test]
    fn test_like_escape() {
        let storage = Storage::open_memory().unwrap();
        let now = chrono::Utc::now().timestamp();

        storage.add_command("test_cmd", None, None, Some(0), None, now, 10, None, None).unwrap();
        storage.add_command("testXcmd", None, None, Some(0), None, now, 10, None, None).unwrap();

        // "test_" should NOT match "testXcmd" since _ is escaped
        let results = storage.get_prefix_matches("test_", 10).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, "test_cmd");
    }

    #[test]
    fn test_delete_command() {
        let storage = Storage::open_memory().unwrap();
        let now = chrono::Utc::now().timestamp();

        storage.add_command("secret_cmd", None, None, Some(0), None, now, 10, None, None).unwrap();
        storage.add_command("ls", None, None, Some(0), None, now, 10, None, None).unwrap();

        assert_eq!(storage.total_commands().unwrap(), 2);
        
        storage.delete_command("secret_cmd").unwrap();
        
        assert_eq!(storage.total_commands().unwrap(), 1);
        let results = storage.get_prefix_matches("", 10).unwrap();
        assert_eq!(results[0].0, "ls");
    }

    #[test]
    fn test_delete_command_preserves_unrelated_base_bigrams() {
        let storage = Storage::open_memory().unwrap();
        let now = chrono::Utc::now().timestamp();

        // Build two independent sequences:
        //   "git status" → "git add ."        (base bigram: "git" → "git add .")
        //   "git diff"   → "git commit -m x"  (base bigram: "git" → "git commit -m x")
        storage.add_command("git add .", None, None, Some(0), None, now, 10, Some("git status"), None).unwrap();
        storage.add_command("git commit -m x", None, None, Some(0), None, now, 10, Some("git diff"), None).unwrap();

        // Both base bigrams should exist before deletion
        let before = storage.get_base_bigram_suggestions("git anything", 10).unwrap();
        assert_eq!(before.len(), 2, "should have 2 base bigrams before delete");

        // Delete "git status" — should only remove base_bigrams where next_cmd = "git status"
        // and should NOT remove unrelated base bigrams like "git" → "git commit -m x"
        storage.delete_command("git status").unwrap();

        let after = storage.get_base_bigram_suggestions("git anything", 10).unwrap();
        // "git" → "git add ." and "git" → "git commit -m x" should BOTH survive
        // because neither has next_cmd = "git status"
        assert_eq!(after.len(), 2, "deleting 'git status' should not touch unrelated base bigrams");
    }

    #[test]
    fn test_delete_command_removes_own_base_bigrams() {
        let storage = Storage::open_memory().unwrap();
        let now = chrono::Utc::now().timestamp();

        // Sequence: "ls" → "git status"  (base bigram: "ls" → "git status")
        storage.add_command("git status", None, None, Some(0), None, now, 10, Some("ls"), None).unwrap();

        let before = storage.get_base_bigram_suggestions("ls", 10).unwrap();
        assert_eq!(before.len(), 1);
        assert_eq!(before[0].0, "git status");

        // Delete "git status" — SHOULD remove "ls" → "git status" base bigram
        storage.delete_command("git status").unwrap();

        let after = storage.get_base_bigram_suggestions("ls", 10).unwrap();
        assert_eq!(after.len(), 0, "base bigram pointing to deleted command should be removed");
    }

    #[test]
    fn test_storage_uses_schema_v2_without_hour_table() {
        let path = temp_db_path("schema-v2");
        let storage = Storage::open(&path).unwrap();

        let version: i32 = storage
            .conn
            .query_row("PRAGMA user_version;", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, 2);

        let hour_table: Option<String> = storage
            .conn
            .query_row(
                "SELECT name FROM sqlite_master WHERE type = 'table' AND name = 'hour_freq'",
                [],
                |row| row.get(0),
            )
            .optional()
            .unwrap();
        assert!(hour_table.is_none(), "schema v2 should not create hour_freq");

        drop(storage);
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn test_storage_recreates_legacy_schema_files() {
        let path = temp_db_path("legacy-recreate");
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(
            "
            CREATE TABLE command_history (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                command TEXT NOT NULL,
                directory TEXT,
                git_branch TEXT,
                timestamp INTEGER NOT NULL,
                exit_code INTEGER,
                session_id TEXT
            );
            CREATE TABLE hour_freq (
                hour INTEGER NOT NULL,
                command TEXT NOT NULL,
                count INTEGER DEFAULT 1,
                PRIMARY KEY (hour, command)
            );
            PRAGMA user_version = 1;
            ",
        )
        .unwrap();
        drop(conn);

        let storage = Storage::open(&path).unwrap();
        let version: i32 = storage
            .conn
            .query_row("PRAGMA user_version;", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, 2);

        let columns: Vec<String> = {
            let mut stmt = storage.conn.prepare("PRAGMA table_info(command_history);").unwrap();
            stmt.query_map([], |row| row.get(1))
                .unwrap()
                .collect::<Result<Vec<String>>>()
                .unwrap()
        };
        assert_eq!(columns, vec!["id", "command", "directory", "timestamp"]);

        drop(storage);
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn test_storage_uses_delete_journal_mode() {
        let path = temp_db_path("journal");
        let storage = Storage::open(&path).unwrap();
        let journal_mode: String = storage
            .conn
            .query_row("PRAGMA journal_mode;", [], |row| row.get(0))
            .unwrap();

        assert_eq!(journal_mode.to_lowercase(), "delete");

        drop(storage);
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }
}
