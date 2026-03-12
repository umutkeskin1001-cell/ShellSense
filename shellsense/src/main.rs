use clap::{Parser, Subcommand};
use shellsense::config::Config;
use shellsense::importer::Importer;
use shellsense::ranker::Ranker;
use shellsense::fuzzy::FuzzyCorrector;
use shellsense::storage::Storage;

#[derive(Parser)]
#[command(
    name = "shellsense",
    about = "🧠 Offline terminal autocomplete — learns your command patterns",
    version
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Record a command execution
    Add {
        /// The command that was executed
        #[arg(long)]
        cmd: String,
        /// Working directory
        #[arg(long)]
        dir: Option<String>,
        /// Git branch name
        #[arg(long)]
        git: Option<String>,
        /// Exit code of the command
        #[arg(long)]
        exit: Option<i32>,
        /// Shell session ID
        #[arg(long)]
        session: Option<String>,
        /// Previous command (for bigram tracking)
        #[arg(long)]
        prev: Option<String>,
        /// Command before previous (for trigram tracking)
        #[arg(long)]
        prev2: Option<String>,
    },

    /// Get smart suggestions
    Suggest {
        /// Current input prefix
        #[arg(long, default_value = "")]
        prefix: String,
        /// Previous command
        #[arg(long)]
        prev: Option<String>,
        /// Command before previous
        #[arg(long)]
        prev2: Option<String>,
        /// Current directory
        #[arg(long)]
        dir: Option<String>,
        /// Active environment contexts (e.g. VIRTUAL_ENV)
        #[arg(long)]
        env: Option<Vec<String>>,
        /// Number of suggestions to return
        #[arg(long, short = 'n')]
        count: Option<usize>,
    },

    /// Get typo correction for a command
    Correct {
        /// The potentially mistyped command
        #[arg(long)]
        input: String,
    },

    /// Import existing shell history
    Import {
        /// Import from bash instead of zsh
        #[arg(long)]
        bash: bool,
        /// Import from fish
        #[arg(long)]
        fish: bool,
    },

    /// Show learning statistics
    Stats,

    /// Check whether the background daemon is currently reachable
    Ping,

    /// Output shell integration script
    Init {
        /// Which shell to initialize (zsh, bash, fish)
        #[arg()]
        shell: Option<String>,
    },

    /// Uninstall ShellSense (removes DB, config, and shell hooks)
    Uninstall,

    /// Run the background daemon server
    Daemon {
        /// Stop the background daemon gracefully
        #[arg(long)]
        stop: bool,
    },

    /// Open the interactive TUI Dashboard
    Dashboard,

    /// Reset all learned data (WARNING: irreversible)
    Reset {
        /// Skip confirmation prompt
        #[arg(long)]
        force: bool,
    },
}

/// Try to send a request to the daemon, auto-spawning it if needed.
/// Uses retry with backoff instead of a fixed sleep.
fn daemon_send_with_retry(req: &shellsense::daemon::Request) -> Result<shellsense::daemon::Response, String> {
    // First attempt
    if let Ok(res) = shellsense::daemon::Daemon::client_send(req) {
        return Ok(res);
    }

    // Daemon not running — spawn it
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::process::Command::new(exe)
            .arg("daemon")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
    }

    // Retry with exponential backoff: 20ms, 40ms, 80ms, 160ms (total ~300ms max)
    for delay_ms in [20, 40, 80, 160] {
        std::thread::sleep(std::time::Duration::from_millis(delay_ms));
        if let Ok(res) = shellsense::daemon::Daemon::client_send(req) {
            return Ok(res);
        }
    }

    Err("Could not connect to daemon after retries".to_string())
}

fn main() {
    let cli = Cli::parse();
    let config = Config::load();
    let db_path = Config::db_path();

    let get_storage = || -> Storage {
        if let Err(e) = Config::ensure_data_dir() {
            eprintln!("Error creating data directory: {}", e);
            std::process::exit(1);
        }

        match Storage::open(&db_path) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("Error opening database: {}", e);
                std::process::exit(1);
            }
        }
    };

    match cli.command {
        Commands::Add { cmd, dir, git, exit, session, prev, prev2 } => {
            // Check privacy filter
            if config.should_exclude(&cmd) {
                return;
            }

            let now = chrono::Local::now();
            let timestamp = now.timestamp();
            let hour = chrono::Timelike::hour(&now);

            let req = shellsense::daemon::Request::Add {
                cmd: cmd.clone(),
                dir: dir.clone(),
                git: git.clone(),
                exit,
                session: session.clone(),
                prev: prev.clone(),
                prev2: prev2.clone(),
                timestamp: Some(timestamp),
                hour: Some(hour),
            };

            if daemon_send_with_retry(&req).is_err() {
                // Fallback to local SQLite
                let storage = get_storage();
                let _ = storage.add_command(&cmd, dir.as_deref(), git.as_deref(), exit, session.as_deref(), timestamp, hour, prev.as_deref(), prev2.as_deref());
            }
        }

        Commands::Suggest { prefix, prev, prev2, dir, env, count } => {
            let req = shellsense::daemon::Request::Suggest {
                prefix: if prefix.is_empty() { None } else { Some(prefix.clone()) },
                prev: prev.clone(),
                prev2: prev2.clone(),
                dir: dir.clone(),
                env: env.clone(),
                count,
                plain: None,
            };

            match daemon_send_with_retry(&req) {
                Ok(shellsense::daemon::Response::Suggestions(cmds)) => {
                    for cmd in cmds {
                        println!("{}", cmd);
                    }
                }
                _ => {
                    // Fallback to local SQLite processing
                    let storage = get_storage();
                    let mut cfg = config;
                    if let Some(n) = count {
                        cfg.general.max_suggestions = n;
                    }

                    let ranker = Ranker::new(cfg);
                    let suggestions = ranker.suggest(
                        &storage,
                        if prefix.is_empty() { None } else { Some(&prefix) },
                        prev.as_deref(),
                        prev2.as_deref(),
                        dir.as_deref(),
                        env.as_deref(),
                    );

                    for suggestion in suggestions {
                        println!("{}", suggestion.command);
                    }
                }
            }
        }

        Commands::Correct { input } => {
            if !db_path.exists() {
                return;
            }

            let storage = get_storage();
            let corrector = FuzzyCorrector::new();
            match storage.get_all_commands(1000) {
                Ok(known) => {
                    if let Some(correction) = corrector.correct(&input, &known) {
                        println!("{}", correction.corrected);
                    }
                }
                Err(e) => {
                    eprintln!("Error: {}", e);
                    std::process::exit(1);
                }
            }
        }

        Commands::Import { bash, fish } => {
            let storage = get_storage();
            if bash {
                println!("🧠 ShellSense — Importing bash history...");
                match Importer::import_bash_history(&storage) {
                    Ok((total, patterns)) => {
                        println!("✅ Imported {} commands, built {} sequence patterns", total, patterns);
                    }
                    Err(e) => {
                        eprintln!("❌ Import failed: {}", e);
                        std::process::exit(1);
                    }
                }
            } else if fish {
                println!("🧠 ShellSense — Importing fish history...");
                match Importer::import_fish_history(&storage) {
                    Ok((total, patterns)) => {
                        println!("✅ Imported {} commands, built {} sequence patterns", total, patterns);
                    }
                    Err(e) => {
                        eprintln!("❌ Import failed: {}", e);
                        std::process::exit(1);
                    }
                }
            } else {
                println!("🧠 ShellSense — Importing zsh history...");
                match Importer::import_zsh_history(&storage) {
                    Ok((total, patterns)) => {
                        println!("✅ Imported {} commands, built {} sequence patterns", total, patterns);
                    }
                    Err(e) => {
                        eprintln!("❌ Import failed: {}", e);
                        std::process::exit(1);
                    }
                }
            }
        }

        Commands::Stats => {
            println!("🧠 ShellSense Statistics");
            println!("━━━━━━━━━━━━━━━━━━━━━━━━");

            if db_path.exists() {
                let storage = get_storage();

                match storage.total_commands() {
                    Ok(total) => println!("📝 Total commands recorded:   {}", total),
                    Err(_) => println!("📝 Total commands:            N/A"),
                }

                match storage.unique_commands() {
                    Ok(unique) => println!("🔤 Unique commands:           {}", unique),
                    Err(_) => println!("🔤 Unique commands:           N/A"),
                }

                match storage.total_patterns() {
                    Ok((bi, tri)) => {
                        println!("🔗 Bigram patterns:           {}", bi);
                        println!("🔗 Trigram patterns:          {}", tri);
                    }
                    Err(_) => println!("🔗 Patterns:                  N/A"),
                }

                if let Ok(size) = storage.db_size_bytes() {
                    let size_kb = size as f64 / 1024.0;
                    if size_kb > 1024.0 {
                        println!("💾 Database size:             {:.1} MB", size_kb / 1024.0);
                    } else {
                        println!("💾 Database size:             {:.0} KB", size_kb);
                    }
                }

                // Show top commands
                match storage.get_top_commands(5) {
                    Ok(top) if !top.is_empty() => {
                        println!();
                        println!("🏆 Top Commands");
                        println!("────────────────────────");
                        for (i, (cmd, count)) in top.iter().enumerate() {
                            println!("   {}. {} (×{})", i + 1, cmd, count);
                        }
                    }
                    _ => {
                        println!();
                        println!("💡 No data yet — start typing commands!");
                    }
                }
            } else {
                println!("📝 Total commands recorded:   0");
                println!("🔤 Unique commands:           0");
                println!("🔗 Bigram patterns:           0");
                println!("🔗 Trigram patterns:          0");
                println!();
                println!("💡 No data yet — start typing commands!");
            }

            println!("📁 Database:                  {}", db_path.display());
            println!("⚙️  Config:                    {}", Config::config_path().display());
        }

        Commands::Ping => {
            if shellsense::daemon::Daemon::is_running() {
                println!("ShellSense daemon is running.");
            } else {
                eprintln!("ShellSense daemon is not running.");
                std::process::exit(1);
            }
        }

        Commands::Init { shell } => {
            match shell.as_deref() {
                Some("zsh") => {
                    print!("{}", include_str!("../shell/shellsense.zsh"));
                }
                Some("bash") => {
                    print!("{}", include_str!("../shell/shellsense.bash"));
                }
                Some("fish") => {
                    print!("{}", include_str!("../shell/shellsense.fish"));
                }
                Some(other) => {
                    eprintln!("❌ Unsupported shell: {}. Supported: zsh, bash, fish", other);
                    std::process::exit(1);
                }
                None => {
                    // Auto-detect shell and print usage hint
                    let detected = std::env::var("SHELL").unwrap_or_default();
                    let shell_name = detected.rsplit('/').next().unwrap_or("unknown");

                    println!("🧠 ShellSense — Shell Integration Setup");
                    println!();

                    match shell_name {
                        "zsh" => {
                            println!("Detected shell: zsh");
                            println!("Add this to your ~/.zshrc:");
                            println!();
                            println!("  eval \"$(shellsense init zsh)\"");
                        }
                        "bash" => {
                            println!("Detected shell: bash");
                            println!("Add this to your ~/.bashrc:");
                            println!();
                            println!("  eval \"$(shellsense init bash)\"");
                        }
                        "fish" => {
                            println!("Detected shell: fish");
                            println!("Add this to your ~/.config/fish/config.fish:");
                            println!();
                            println!("  shellsense init fish | source");
                        }
                        _ => {
                            println!("Could not detect shell. Add one of these to your shell config:");
                            println!();
                            println!("  # Zsh (~/.zshrc):");
                            println!("  eval \"$(shellsense init zsh)\"");
                            println!();
                            println!("  # Bash (~/.bashrc):");
                            println!("  eval \"$(shellsense init bash)\"");
                            println!();
                            println!("  # Fish (~/.config/fish/config.fish):");
                            println!("  shellsense init fish | source");
                        }
                    }

                }
            }
        }

        Commands::Uninstall => {
            println!("🧠 ShellSense — Uninstalling...");

            if let Some(home) = dirs::home_dir() {
                // 1. Remove Zsh hook
                let zshrc = home.join(".zshrc");
                if zshrc.exists() {
                    if let Ok(content) = std::fs::read_to_string(&zshrc) {
                        let new_content = shellsense::shell::remove_init_loader_lines(&content);
                        if new_content != content {
                            let _ = std::fs::write(&zshrc, new_content);
                            println!("✅ Removed hook from ~/.zshrc");
                        }
                    }
                }

                // 2. Remove Bash hook
                let bashrc = home.join(".bashrc");
                if bashrc.exists() {
                    if let Ok(content) = std::fs::read_to_string(&bashrc) {
                        let new_content = shellsense::shell::remove_init_loader_lines(&content);
                        if new_content != content {
                            let _ = std::fs::write(&bashrc, new_content);
                            println!("✅ Removed hook from ~/.bashrc");
                        }
                    }
                }

                // 3. Remove Fish hook
                let fish_config = home.join(".config/fish/config.fish");
                if fish_config.exists() {
                    if let Ok(content) = std::fs::read_to_string(&fish_config) {
                        let new_content = shellsense::shell::remove_init_loader_lines(&content);
                        if new_content != content {
                            let _ = std::fs::write(&fish_config, new_content);
                            println!("✅ Removed hook from ~/.config/fish/config.fish");
                        }
                    }
                }
            }

            // 4. Stop daemon
            if shellsense::daemon::Daemon::is_running() {
                let _ = shellsense::daemon::Daemon::client_send(&shellsense::daemon::Request::Shutdown);
                println!("✅ Stopped daemon");
            }

            // 5. Remove ~/.shellsense
            let data_dir = Config::data_dir();
            if data_dir.exists() {
                match std::fs::remove_dir_all(&data_dir) {
                    Ok(_) => println!("✅ Removed database and config from {}", data_dir.display()),
                    Err(e) => eprintln!("❌ Failed to remove {}: {}", data_dir.display(), e),
                }
            }

            println!("👋 ShellSense has been successfully uninstalled.");
            println!("   (Note: Use 'cargo uninstall shellsense' or rm ~/.cargo/bin/shellsense to remove the binary)");
        }

        Commands::Daemon { stop } => {
            if stop {
                if shellsense::daemon::Daemon::is_running() {
                    println!("🧠 ShellSense — Stopping daemon...");
                    match shellsense::daemon::Daemon::client_send(&shellsense::daemon::Request::Shutdown) {
                        Ok(_) => println!("✅ Daemon stopped successfully."),
                        Err(e) => eprintln!("❌ Failed to send shutdown request: {}", e),
                    }
                } else {
                    println!("💡 Daemon is not running.");
                }
                std::process::exit(0);
            }

            let daemon = shellsense::daemon::Daemon::new();
            if let Err(e) = daemon.run() {
                eprintln!("Daemon error: {}", e);
                std::process::exit(1);
            }
        }

        Commands::Dashboard => {
            if let Err(e) = shellsense::tui::run_dashboard() {
                eprintln!("Dashboard error: {}", e);
                std::process::exit(1);
            }
        }

        Commands::Reset { force } => {
            if !force {
                println!("⚠️  This will delete ALL learned command patterns.");
                println!("   Run with --force to confirm: shellsense reset --force");
                return;
            }

            if !db_path.exists() {
                println!("💡 No learned data found.");
                return;
            }

            let storage = get_storage();

            match storage.reset() {
                Ok(_) => println!("✅ All learned data has been reset."),
                Err(e) => {
                    eprintln!("❌ Reset failed: {}", e);
                    std::process::exit(1);
                }
            }
        }
    }
}
