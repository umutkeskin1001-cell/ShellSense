#!/usr/bin/env bash
# ShellSense — Offline Terminal AI Autocomplete (Bash Plugin)

# ── Binary resolution ──
if command -v shellsense &>/dev/null; then
    _SHELLSENSE_BIN="shellsense"
elif [[ -x "$HOME/.shellsense/bin/shellsense" ]]; then
    _SHELLSENSE_BIN="$HOME/.shellsense/bin/shellsense"
elif [[ -x "$HOME/.cargo/bin/shellsense" ]]; then
    _SHELLSENSE_BIN="$HOME/.cargo/bin/shellsense"
else
    echo "[shellsense] Binary not found."
    return 1
fi

_SHELLSENSE_SESSION="$$"
_SHELLSENSE_CMD_1AGO=""
_SHELLSENSE_CMD_2AGO=""
_SHELLSENSE_LAST_CMD=""
_SHELLSENSE_SOCK="$HOME/.shellsense/daemon.sock"

# ── Ensure daemon is running ──
if ! echo '{"Ping"}' | nc -U "$_SHELLSENSE_SOCK" 2>/dev/null | grep -q Pong; then
    "$_SHELLSENSE_BIN" daemon &
fi

# ── Bash precmd (PROMPT_COMMAND) ──
_shellsense_prompt_command() {
    local exit_code=$?
    # Get last command from history
    local last_hist
    last_hist=$(history 1 | sed -e "s/^[ ]*[0-9]*[ ]*//")
    _SHELLSENSE_LAST_CMD="$last_hist"

    [[ -z "$_SHELLSENSE_LAST_CMD" ]] && return

    # Guard: don't re-record the same command on empty Enter
    [[ "$_SHELLSENSE_LAST_CMD" == "$_SHELLSENSE_CMD_1AGO" ]] && return

    local cmd_esc="${_SHELLSENSE_LAST_CMD//\\/\\\\}"
    cmd_esc="${cmd_esc//\"/\\\"}"
    cmd_esc="${cmd_esc//$'\n'/\\n}"
    cmd_esc="${cmd_esc//$'\r'/\\r}"
    cmd_esc="${cmd_esc//$'\t'/\\t}"
    local dir_esc=${PWD//\"/\\\"}
    local ts=$(date +%s)
    local hour=$((10#$(date +%H)))
    
    local json="{\"Add\":{\"cmd\":\"$cmd_esc\",\"dir\":\"$dir_esc\",\"exit\":$exit_code,\"session\":\"$_SHELLSENSE_SESSION\",\"timestamp\":$ts,\"hour\":$hour"
    
    if [[ -d .git ]] || git rev-parse --git-dir &>/dev/null 2>&1; then
        local b; b=$(git branch --show-current 2>/dev/null)
        if [[ -n "$b" ]]; then
            json+=",\"git\":\"$b\""
        fi
    fi
    
    if [[ -n "$_SHELLSENSE_CMD_1AGO" ]]; then
        local p1="${_SHELLSENSE_CMD_1AGO//\\/\\\\}"; p1="${p1//\"/\\\"}"; p1="${p1//$'\n'/\\n}"; json+=",\"prev\":\"$p1\""
    fi
    if [[ -n "$_SHELLSENSE_CMD_2AGO" ]]; then
        local p2="${_SHELLSENSE_CMD_2AGO//\\/\\\\}"; p2="${p2//\"/\\\"}"; p2="${p2//$'\n'/\\n}"; json+=",\"prev2\":\"$p2\""
    fi
    
    json+="}}"

    echo "$json" | nc -U "$_SHELLSENSE_SOCK" &>/dev/null &

    _SHELLSENSE_CMD_2AGO="$_SHELLSENSE_CMD_1AGO"
    _SHELLSENSE_CMD_1AGO="$_SHELLSENSE_LAST_CMD"
}

# ── Bindings & Autocomplete ──
# Bash's Readline natively doesn't support async ghost text as easily as Zsh ZLE.
# A full readline implementation requires a C extension or `bind -x`.
# For V1 of the bash integration, we inject a basic history search override.

_shellsense_suggest_bash() {
    local prefix="${READLINE_LINE:0:$READLINE_POINT}"
    if [[ ${#prefix} -lt 2 ]]; then return; fi

    local pfx_esc="${prefix//\\/\\\\}"
    pfx_esc="${pfx_esc//\"/\\\"}"
    pfx_esc="${pfx_esc//$'\n'/\\n}"
    pfx_esc="${pfx_esc//$'\r'/\\r}"
    pfx_esc="${pfx_esc//$'\t'/\\t}"
    local dir_esc=${PWD//\"/\\\"}
    local json="{\"Suggest\":{\"prefix\":\"$pfx_esc\",\"dir\":\"$dir_esc\",\"count\":1,\"plain\":true"
    
    if [[ -n "$_SHELLSENSE_CMD_1AGO" ]]; then
        local p1="${_SHELLSENSE_CMD_1AGO//\\/\\\\}"; p1="${p1//\"/\\\"}"; p1="${p1//$'\n'/\\n}"
        json+=",\"prev\":\"$p1\""
    fi
    if [[ -n "$_SHELLSENSE_CMD_2AGO" ]]; then
        local p2="${_SHELLSENSE_CMD_2AGO//\\/\\\\}"; p2="${p2//\"/\\\"}"; p2="${p2//$'\n'/\\n}"
        json+=",\"prev2\":\"$p2\""
    fi
    
    local has_env=0
    if [[ -n "$VIRTUAL_ENV" || -n "$KUBECONFIG" || -n "$AWS_PROFILE" ]]; then
        json+=",\"env\":["
        [[ -n "$VIRTUAL_ENV" ]] && json+="\"VIRTUAL_ENV\""; has_env=1
        if [[ -n "$KUBECONFIG" ]]; then
            [[ $has_env -eq 1 ]] && json+=","
            json+="\"KUBECONFIG\""; has_env=1
        fi
        if [[ -n "$AWS_PROFILE" ]]; then
            [[ $has_env -eq 1 ]] && json+=","
            json+="\"AWS_PROFILE\""
        fi
        json+="]"
    fi
    json+="}}"

    local raw
    if command -v nc >/dev/null; then
        raw=$(echo "$json" | nc -U "$_SHELLSENSE_SOCK" 2>/dev/null)
    else
        return
    fi
    
    if [[ -z "$raw" || "$raw" == '{"Error"'* ]]; then return; fi

    # read first line
    IFS=$'\n' read -r suggestion <<< "$raw"

    if [[ -n "$suggestion" && "$suggestion" != "$prefix" ]]; then
        READLINE_LINE="$suggestion"
        READLINE_POINT=${#READLINE_LINE}
    fi
}

bind -x '"\C-e": _shellsense_suggest_bash'

if [[ -z "${PROMPT_COMMAND:-}" ]]; then
    PROMPT_COMMAND="_shellsense_prompt_command"
else
    # Append to existing
    PROMPT_COMMAND="${PROMPT_COMMAND%;}; _shellsense_prompt_command"
fi
