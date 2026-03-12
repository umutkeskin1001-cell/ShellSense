# 🧠 ShellSense

**Zero-Latency, Offline Terminal AI Autocomplete**
ShellSense learns your command patterns locally and suggests next steps natively inside your shell. It acts as an Intelligent History engine powered by Markov chains and QWERTY-aware fuzzy matching, keeping all your data safely on your machine without leaving a footprint.

![MIT License](https://img.shields.io/badge/license-MIT-blue.svg)
![Rust](https://img.shields.io/badge/built%20with-Rust-orange.svg)

## ⚡ What Makes ShellSense V2 Different?

1. **True Zero-Fork Latency**: Most shell completion tools fork a Python or Node process on every keystroke. ShellSense uses a blazing-fast background Rust daemon, communicating via native Unix Domain Sockets (`nc -U`). Ghost text completion arrives in under **0.05ms**, eliminating all terminal stutter.
2. **Absolute Offline Privacy**: Your `.env` secrets never leave your laptop. There are no API keys, no network boundaries, and no LLMs.
3. **QWERTY-Aware Fuzzy Typos**: Typing `fkir` instead of `flir`? ShellSense knows `k` and `l` are physically adjacent on your keyboard and automatically fixes mechanical mistakes instantly.
4. **Adaptive Context**: Typing inside a Python virtual environment? ShellSense detects `$VIRTUAL_ENV` implicitly and statistically boosts commands like `pip` and `python` to the top.
5. **Interactive TUI Dashboard**: Integrated telemetry allows you to view and selectively purge unwanted commands from all AI models via `shellsense dashboard`.

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

*(Note: ShellSense uses standard `nc` via `-U` flag under the hood on macOS and Linux for immediate JSON IPC).*

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

## 💾 Syncing Across Machines

Because ShellSense is entirely offline, its brain lives in a single SQLite database file: `~/.shellsense/history.db`.

To sync your autocomplete model between your work MacBook and personal Linux machine, simply symlink this file to Dropbox, iCloud, or a private Git repository:
```bash
mv ~/.shellsense/history.db ~/Dropbox/shellsense.db
ln -s ~/Dropbox/shellsense.db ~/.shellsense/history.db
```

## ⚙️ Configuration

Customize the weights to your liking locally at `~/.shellsense/config.toml`:

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
