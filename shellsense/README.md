# 🧠 ShellSense

**Offline terminal autocomplete that learns locally**
ShellSense learns your command patterns on your machine and suggests likely next steps inside your shell. It uses local history, sequence ranking, and QWERTY-aware fuzzy matching to stay private and fast without calling external APIs.

![MIT License](https://img.shields.io/badge/license-MIT-blue.svg)
![Rust](https://img.shields.io/badge/built%20with-Rust-orange.svg)

## ⚡ What Makes ShellSense Different?

1. **Offline by default**: Your history stays local. There are no API keys, no network calls, and no LLM dependencies.
2. **Rust daemon + SQLite storage**: ShellSense keeps ranking state in a local SQLite database and serves suggestions through a background daemon over a Unix socket.
3. **QWERTY-aware typo correction**: Mechanical typos like `gti` or adjacent-key slips can still resolve to commands you actually use.
4. **Context-aware ranking**: Previous commands, current prefix, and directory-specific habits all influence suggestion order.
5. **Inspectable local state**: You can check stats, review top commands, and remove noisy entries from the local model.

## 🚀 Quick Start

ShellSense compiles natively to your architecture.

```bash
# 1. Install to your cargo path
cargo install --path .

# 2. Add to your Shell configuration (~/.zshrc, ~/.bashrc, or ~/.config/fish/config.fish)
# For Zsh:
eval "$(shellsense init zsh)"

# For Bash:
eval "$(shellsense init bash)"

# For Fish:
shellsense init fish | source
```

Re-source your shell. Start typing.

Shell support today:
- `zsh`: primary and most polished experience
- `bash`: supported with a lighter inline suggestion flow
- `fish`: best-effort support

Note:
- ShellSense currently uses standard `nc -U` shell calls to talk to the daemon on macOS and Linux.
- `shellsense init <shell>` only prints shell integration code. It does not create config files.

## ⌨️ Mac-Friendly Keybindings

ShellSense bindings are natively tuned for standard Unix/macOS terminal emulators:

| Key Binding | Action |
|---|---|
| `Tab` | Accept suggestion (falls back to native completion) |
| `→` (Right arrow) | Accept suggestion (falls back to native forward char) |
| `Ctrl+E` | Accept suggestion |
| `Ctrl+Space` | Accept one word / chunk |
| `Escape` | Dismiss suggestion |

## 📊 How The Markov Engine Works

ShellSense is an **Intelligent History Engine**, not a Generative LLM. If you have never typed a specific complex `tar` pipeline, ShellSense will not magically write it for you. 

Instead, it perfectly masters your actual workflows using a multi-signal scoring ranker:
1. **Sequence** — "After `docker build`, you usually run `docker run`"
2. **Prefix** — What are you typing right now?
3. **Typo Correction** — Keyboard-distance Levenshtein mapping.
4. **Directory** — Unique workflows isolated per folder.
5. **Context Variables** — e.g., Kubernetes `$KUBECONFIG` profiles.

## 📁 Local Files

ShellSense keeps its data in `~/.shellsense` by default, or in `$SHELLSENSE_DATA_DIR` if you override it.

Files you may see:
- `history.db` — the SQLite learning database
- `daemon.sock` — the Unix socket used while the daemon is running
- `config.toml` — optional config file if you create one manually

## 💾 Syncing Across Machines

Because ShellSense is entirely offline, its brain lives in a single SQLite database file: `~/.shellsense/history.db`.

To sync your autocomplete model between your work MacBook and personal Linux machine, simply symlink this file to Dropbox, iCloud, or a private Git repository:
```bash
mv ~/.shellsense/history.db ~/Dropbox/shellsense.db
ln -s ~/Dropbox/shellsense.db ~/.shellsense/history.db
```

## ⚙️ Configuration

Customize the weights by creating `~/.shellsense/config.toml` (or `$SHELLSENSE_DATA_DIR/config.toml`):

```toml
[general]
max_suggestions = 5

[weights]
sequence = 0.40
prefix = 0.25
frequency = 0.15
recency = 0.10
directory = 0.10

[privacy]
# Commands matching these patterns will NOT be recorded
exclude_patterns = ["*password*", "*secret*", "*token*", "*AWS_SECRET*"]
```

## 📜 License
MIT
