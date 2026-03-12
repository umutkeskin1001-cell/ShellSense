use std::fs;
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_shellsense")
}

fn unique_temp_dir(label: &str) -> PathBuf {
    let mut path = std::env::temp_dir();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock before unix epoch")
        .as_nanos();
    path.push(format!(
        "shellsense-{label}-{}-{nanos}",
        std::process::id()
    ));
    path
}

fn run(data_dir: &Path, args: &[&str]) -> Output {
    Command::new(binary())
        .args(args)
        .env("SHELLSENSE_DATA_DIR", data_dir)
        .output()
        .expect("failed to run shellsense")
}

fn unix_sockets_supported() -> bool {
    let data_dir = unique_temp_dir("uds-check");
    let _ = fs::create_dir_all(&data_dir);
    let socket_path = data_dir.join("daemon.sock");
    let result = UnixListener::bind(&socket_path).is_ok();
    let _ = fs::remove_file(&socket_path);
    let _ = fs::remove_dir_all(&data_dir);
    result
}

fn spawn_daemon(data_dir: &Path) -> Child {
    Command::new(binary())
        .arg("daemon")
        .env("SHELLSENSE_DATA_DIR", data_dir)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("failed to spawn daemon")
}

fn wait_for_ping(data_dir: &Path, timeout: Duration) -> bool {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if run(data_dir, &["ping"]).status.success() {
            return true;
        }
        thread::sleep(Duration::from_millis(25));
    }
    false
}

fn wait_for_exit(child: &mut Child, timeout: Duration) -> Option<ExitStatus> {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if let Ok(Some(status)) = child.try_wait() {
            return Some(status);
        }
        thread::sleep(Duration::from_millis(25));
    }
    None
}

fn stop_daemon(data_dir: &Path) {
    let _ = run(data_dir, &["daemon", "--stop"]);
}

#[test]
fn read_only_commands_do_not_create_data_dir() {
    let cases: &[(&str, &[&str], bool)] = &[
        ("init", &["init", "zsh"], true),
        ("ping", &["ping"], false),
        ("stats", &["stats"], true),
        ("correct", &["correct", "--input", "git"], true),
        ("dashboard", &["dashboard"], true),
    ];

    for (label, args, expect_success) in cases {
        let data_dir = unique_temp_dir(label);
        let output = run(&data_dir, args);

        assert_eq!(
            output.status.success(),
            *expect_success,
            "unexpected exit status for {:?}: stdout={}, stderr={}",
            args,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            !data_dir.exists(),
            "{label} should not create the data directory"
        );
    }
}

#[test]
fn writes_respect_shellsense_data_dir_override() {
    let data_dir = unique_temp_dir("override");
    let add = run(&data_dir, &["add", "--cmd", "ls"]);
    assert!(
        add.status.success(),
        "add failed: stdout={}, stderr={}",
        String::from_utf8_lossy(&add.stdout),
        String::from_utf8_lossy(&add.stderr)
    );

    assert!(
        data_dir.join("history.db").exists(),
        "writes should create the database inside SHELLSENSE_DATA_DIR"
    );

    let _ = fs::remove_dir_all(&data_dir);
}

#[test]
fn ping_succeeds_for_live_daemon_and_second_start_exits_cleanly() {
    if !unix_sockets_supported() {
        return;
    }

    let data_dir = unique_temp_dir("daemon-running");
    let mut first = spawn_daemon(&data_dir);
    assert!(
        wait_for_ping(&data_dir, Duration::from_secs(3)),
        "daemon never became reachable"
    );

    let ping = run(&data_dir, &["ping"]);
    assert!(
        ping.status.success(),
        "ping failed: stdout={}, stderr={}",
        String::from_utf8_lossy(&ping.stdout),
        String::from_utf8_lossy(&ping.stderr)
    );

    let mut second = spawn_daemon(&data_dir);
    let second_status = wait_for_exit(&mut second, Duration::from_secs(2))
        .expect("second daemon should exit quickly when one is already running");
    assert!(
        second_status.success(),
        "second daemon should treat an existing daemon as success"
    );

    stop_daemon(&data_dir);
    let first_status = wait_for_exit(&mut first, Duration::from_secs(3))
        .expect("first daemon did not exit after stop");
    assert!(first_status.success(), "first daemon exited unsuccessfully");

    let _ = fs::remove_dir_all(&data_dir);
}

#[test]
fn daemon_recovers_from_stale_socket_file() {
    if !unix_sockets_supported() {
        return;
    }

    let data_dir = unique_temp_dir("stale-socket");
    fs::create_dir_all(&data_dir).expect("failed to create temp data dir");
    fs::write(data_dir.join("daemon.sock"), b"stale").expect("failed to create stale socket file");

    let mut child = spawn_daemon(&data_dir);
    assert!(
        wait_for_ping(&data_dir, Duration::from_secs(3)),
        "daemon did not recover from a stale socket file"
    );

    stop_daemon(&data_dir);
    let status = wait_for_exit(&mut child, Duration::from_secs(3))
        .expect("daemon did not exit after stop");
    assert!(status.success(), "daemon exited unsuccessfully");

    let _ = fs::remove_dir_all(&data_dir);
}
