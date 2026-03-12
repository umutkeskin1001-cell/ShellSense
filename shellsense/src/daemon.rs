use crate::config::Config;
use crate::ranker::Ranker;
use crate::storage::Storage;
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::sync::mpsc::{self, Sender, Receiver};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

/// IPC RPC Protocol Messages
#[derive(Serialize, Deserialize, Debug)]
pub enum Request {
    Ping,
    Add {
        cmd: String,
        dir: Option<String>,
        git: Option<String>,
        exit: Option<i32>,
        session: Option<String>,
        prev: Option<String>,
        prev2: Option<String>,
        timestamp: i64,
        hour: u32,
    },
    Suggest {
        prefix: Option<String>,
        prev: Option<String>,
        prev2: Option<String>,
        dir: Option<String>,
        env: Option<Vec<String>>,
        count: Option<usize>,
        plain: Option<bool>,
    },
    Shutdown,
}

/// Message enum for the background database writer thread
#[derive(Debug)]
enum DbMessage {
    Add {
        cmd: String,
        dir: Option<String>,
        git: Option<String>,
        exit: Option<i32>,
        session: Option<String>,
        prev: Option<String>,
        prev2: Option<String>,
        timestamp: i64,
        hour: u32,
    },
    Stop,
}

#[derive(Serialize, Deserialize, Debug)]
pub enum Response {
    Pong,
    Ok,
    Suggestions(Vec<String>),
    PlainText(String),
    Error(String),
}

/// The background daemon that holds the SQLite connection open
pub struct Daemon {
    config: Config,
    socket_path: PathBuf,
}

impl Default for Daemon {
    fn default() -> Self {
        Self::new()
    }
}

impl Daemon {
    pub fn new() -> Self {
        let config = Config::load();
        let socket_path = Config::data_dir().join("daemon.sock");
        Daemon { config, socket_path }
    }

    pub fn run(&self) -> Result<(), Box<dyn std::error::Error>> {
        // Clean up stale socket
        if self.socket_path.exists() {
            std::fs::remove_file(&self.socket_path)?;
        }

        let listener = UnixListener::bind(&self.socket_path)?;

        // Strict socket permissions (0600)
        let mut perms = std::fs::metadata(&self.socket_path)?.permissions();
        perms.set_mode(0o600);
        std::fs::set_permissions(&self.socket_path, perms)?;

        // Install signal handler for clean socket removal on SIGTERM/SIGINT
        let shutdown_flag = Arc::new(AtomicBool::new(false));
        let signal_flag = Arc::clone(&shutdown_flag);

        // Register ctrlc/SIGTERM handler
        let _ = ctrlc::set_handler(move || {
            signal_flag.store(true, Ordering::SeqCst);
            // Connect to self to unblock the listener.incoming() loop
            let sock = Config::data_dir().join("daemon.sock");
            let _ = UnixStream::connect(&sock);
        });

        let storage = Arc::new(Mutex::new(Storage::open(Config::db_path())?));
        let ranker = Arc::new(Ranker::new(self.config.clone()));

        // Perform periodic cleanup and vacuum
        if let Ok(db) = storage.lock() {
            let _ = db.vacuum_and_prune(180); // 6 months
        }

        // Setup background channel for async SQLite writes
        let (tx, rx): (Sender<DbMessage>, Receiver<DbMessage>) = mpsc::channel();
        let writer_storage = Arc::clone(&storage);

        let writer_thread = thread::spawn(move || {
            while let Ok(msg) = rx.recv() {
                match msg {
                    DbMessage::Add { cmd, dir, git, exit, session, prev, prev2, timestamp, hour } => {
                        if let Ok(db) = writer_storage.lock() {
                            let _ = db.add_command(&cmd, dir.as_deref(), git.as_deref(), exit, session.as_deref(), timestamp, hour, prev.as_deref(), prev2.as_deref());
                        }
                    }
                    DbMessage::Stop => break,
                }
            }
        });

        // Set listener to non-blocking so we can check shutdown flag
        listener.set_nonblocking(true)?;

        // Main accept loop
        loop {
            if shutdown_flag.load(Ordering::SeqCst) {
                break;
            }

            match listener.accept() {
                Ok((stream, _)) => {
                    // Set stream back to blocking with read timeout
                    stream.set_nonblocking(false).ok();
                    stream.set_read_timeout(Some(Duration::from_secs(5))).ok();

                    let storage = Arc::clone(&storage);
                    let ranker = Arc::clone(&ranker);
                    let flag = Arc::clone(&shutdown_flag);
                    let tx_clone = tx.clone();

                    thread::spawn(move || {
                        if Self::handle_client(stream, storage, ranker, tx_clone) {
                            flag.store(true, Ordering::SeqCst);
                            // Connect to self to unblock the accept loop
                            let sock = Config::data_dir().join("daemon.sock");
                            let _ = UnixStream::connect(&sock);
                        }
                    });
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    // No connection ready, sleep briefly and retry
                    thread::sleep(Duration::from_millis(50));
                    continue;
                }
                Err(err) => eprintln!("Connection failed: {}", err),
            }
        }

        // Clean shutdown
        let _ = tx.send(DbMessage::Stop);
        let _ = writer_thread.join();

        // Remove socket file
        let _ = std::fs::remove_file(&self.socket_path);

        Ok(())
    }

    /// Returns `true` if the daemon should shut down.
    fn handle_client(stream: UnixStream, storage: Arc<Mutex<Storage>>, ranker: Arc<Ranker>, tx: Sender<DbMessage>) -> bool {
        // Limit reads to 64KB to prevent OOM from malicious clients
        const MAX_REQUEST_SIZE: u64 = 64 * 1024;
        let mut buf = String::new();
        if stream.try_clone()
            .map(|s| s.take(MAX_REQUEST_SIZE).read_to_string(&mut buf))
            .is_err()
        {
            return false;
        }
        // Also handle read failure on the take'd stream
        if buf.is_empty() {
            return false;
        }
        let mut stream = stream;

        let request: Request = match serde_json::from_str(&buf) {
            Ok(req) => req,
            Err(e) => {
                let _ = Self::send_response(&mut stream, &Response::Error(e.to_string()));
                return false;
            }
        };

        let mut shutdown = false;

        let response = match request {
            Request::Ping => Response::Pong,
            Request::Shutdown => {
                shutdown = true;
                Response::Ok
            }
            Request::Add { cmd, dir, git, exit, session, prev, prev2, timestamp, hour } => {
                // Instantly pass to background writer queue and return Ok
                let _ = tx.send(DbMessage::Add { cmd, dir, git, exit, session, prev, prev2, timestamp, hour });
                Response::Ok
            }
            Request::Suggest { prefix, prev, prev2, dir, env, count, plain } => {
                let limit = count.unwrap_or(5);
                match storage.lock() {
                    Ok(db) => {
                        let mut suggestions = ranker.suggest(&db, prefix.as_deref(), prev.as_deref(), prev2.as_deref(), dir.as_deref(), env.as_deref());
                        suggestions.truncate(limit);
                        let cmds: Vec<String> = suggestions.into_iter().map(|s| s.command).collect();

                        if plain.unwrap_or(false) {
                            Response::PlainText(cmds.join("\n"))
                        } else {
                            Response::Suggestions(cmds)
                        }
                    }
                    Err(_) => Response::Error("Database lock failed".to_string()),
                }
            }
        };

        let _ = Self::send_response(&mut stream, &response);
        shutdown
    }

    fn send_response(stream: &mut UnixStream, res: &Response) -> std::io::Result<()> {
        match res {
            Response::PlainText(text) => {
                stream.write_all(text.as_bytes())?;
            }
            _ => {
                let json = serde_json::to_string(res)?;
                stream.write_all(json.as_bytes())?;
            }
        }
        stream.shutdown(std::net::Shutdown::Write)
    }

    /// Helper for the CLI to send an IPC request to the daemon
    pub fn client_send(req: &Request) -> Result<Response, String> {
        let socket_path = Config::data_dir().join("daemon.sock");

        let mut stream = UnixStream::connect(&socket_path)
            .map_err(|e| format!("Could not connect to daemon: {}", e))?;

        // Set a read timeout to avoid blocking forever
        stream.set_read_timeout(Some(Duration::from_secs(5))).ok();

        // Write JSON
        let json = serde_json::to_string(req).map_err(|e| e.to_string())?;
        stream.write_all(json.as_bytes()).map_err(|e| e.to_string())?;
        stream.shutdown(std::net::Shutdown::Write).map_err(|e| e.to_string())?;

        // Read response
        let mut buf = String::new();
        stream.read_to_string(&mut buf).map_err(|e| e.to_string())?;

        serde_json::from_str(&buf).map_err(|e| e.to_string())
    }

    /// Checks if the daemon is currently running
    pub fn is_running() -> bool {
        matches!(Self::client_send(&Request::Ping), Ok(Response::Pong))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_request_serialization_roundtrip_ping() {
        let req = Request::Ping;
        let json = serde_json::to_string(&req).unwrap();
        let decoded: Request = serde_json::from_str(&json).unwrap();
        assert!(matches!(decoded, Request::Ping));
    }

    #[test]
    fn test_request_serialization_roundtrip_add() {
        let req = Request::Add {
            cmd: "git status".to_string(),
            dir: Some("/project".to_string()),
            git: Some("main".to_string()),
            exit: Some(0),
            session: Some("12345".to_string()),
            prev: Some("ls".to_string()),
            prev2: None,
            timestamp: 1700000000,
            hour: 14,
        };
        let json = serde_json::to_string(&req).unwrap();
        let decoded: Request = serde_json::from_str(&json).unwrap();
        match decoded {
            Request::Add { cmd, dir, exit, hour, .. } => {
                assert_eq!(cmd, "git status");
                assert_eq!(dir.unwrap(), "/project");
                assert_eq!(exit.unwrap(), 0);
                assert_eq!(hour, 14);
            }
            _ => panic!("expected Request::Add"),
        }
    }

    #[test]
    fn test_request_serialization_roundtrip_suggest() {
        let req = Request::Suggest {
            prefix: Some("git".to_string()),
            prev: None,
            prev2: None,
            dir: Some("/home".to_string()),
            env: Some(vec!["VIRTUAL_ENV".to_string()]),
            count: Some(5),
            plain: Some(true),
        };
        let json = serde_json::to_string(&req).unwrap();
        let decoded: Request = serde_json::from_str(&json).unwrap();
        match decoded {
            Request::Suggest { prefix, env, count, plain, .. } => {
                assert_eq!(prefix.unwrap(), "git");
                assert_eq!(env.unwrap(), vec!["VIRTUAL_ENV"]);
                assert_eq!(count.unwrap(), 5);
                assert!(plain.unwrap());
            }
            _ => panic!("expected Request::Suggest"),
        }
    }

    #[test]
    fn test_response_serialization_roundtrip() {
        let responses = vec![
            Response::Pong,
            Response::Ok,
            Response::Error("test error".to_string()),
            Response::Suggestions(vec!["git status".to_string(), "git add .".to_string()]),
            Response::PlainText("git status\ngit add .".to_string()),
        ];
        for resp in responses {
            let json = serde_json::to_string(&resp).unwrap();
            let _decoded: Response = serde_json::from_str(&json).unwrap();
        }
    }

    #[test]
    fn test_request_with_special_chars() {
        // Commands can contain quotes, backslashes, newlines etc.
        let req = Request::Add {
            cmd: "echo \"hello\\nworld\"".to_string(),
            dir: Some("/path/with spaces/here".to_string()),
            git: None,
            exit: Some(0),
            session: None,
            prev: Some("git commit -m 'test \"quoted\"'".to_string()),
            prev2: None,
            timestamp: 1700000000,
            hour: 10,
        };
        let json = serde_json::to_string(&req).unwrap();
        let decoded: Request = serde_json::from_str(&json).unwrap();
        match decoded {
            Request::Add { cmd, dir, prev, .. } => {
                assert_eq!(cmd, "echo \"hello\\nworld\"");
                assert_eq!(dir.unwrap(), "/path/with spaces/here");
                assert!(prev.unwrap().contains("quoted"));
            }
            _ => panic!("expected Request::Add"),
        }
    }
}
