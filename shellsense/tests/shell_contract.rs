use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn repo_path(path: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(path)
}

fn read_script(path: &str) -> String {
    fs::read_to_string(repo_path(path)).expect("failed to read shell script")
}

#[test]
fn zsh_and_bash_scripts_are_syntax_valid() {
    for (shell, script) in [("zsh", "shell/shellsense.zsh"), ("bash", "shell/shellsense.bash")] {
        let output = Command::new(shell)
            .arg("-n")
            .arg(repo_path(script))
            .output()
            .expect("failed to run shell syntax check");

        assert!(
            output.status.success(),
            "{shell} syntax check failed: stdout={}, stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn shipped_shell_scripts_use_ping_and_centralized_json_escape() {
    for script in [
        "shell/shellsense.zsh",
        "shell/shellsense.bash",
        "shell/shellsense.fish",
    ] {
        let content = read_script(script);

        assert!(
            content.contains("_SHELLSENSE_LOADED"),
            "{script} should guard against double-loading"
        );
        assert!(
            content.contains(" ping"),
            "{script} should use the public ping command"
        );
        assert!(
            content.to_lowercase().contains("json_escape"),
            "{script} should centralize JSON escaping"
        );
        assert!(
            !content.contains("{\"Ping\"}"),
            "{script} should not hand-roll ping IPC"
        );
        assert!(
            !content.contains("grep -q Pong"),
            "{script} should not grep raw daemon responses"
        );
        assert!(
            !content.contains("date +%s"),
            "{script} should not stamp timestamps in the shell"
        );
        assert!(
            !content.contains("git branch --show-current"),
            "{script} should not shell out for git branch metadata"
        );
    }
}
