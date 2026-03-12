#!/usr/bin/env zsh
# ShellSense — Offline Terminal AI Autocomplete (Zsh Plugin)

# ── Binary resolution ──
if command -v shellsense &>/dev/null; then
    _SHELLSENSE_BIN="shellsense"
elif [[ -x "$HOME/.shellsense/bin/shellsense" ]]; then
    _SHELLSENSE_BIN="$HOME/.shellsense/bin/shellsense"
elif [[ -x "$HOME/.cargo/bin/shellsense" ]]; then
    _SHELLSENSE_BIN="$HOME/.cargo/bin/shellsense"
else
    echo "[shellsense] Binary not found. Run: cargo install --path /path/to/shellsense"
    return 1
fi

# ── State ──
_SHELLSENSE_SESSION="$$"
_SHELLSENSE_CMD_1AGO=""
_SHELLSENSE_CMD_2AGO=""
_SHELLSENSE_LAST_CMD=""
_SHELLSENSE_SUGGESTION=""
_SHELLSENSE_IS_CORRECTION=0
_SHELLSENSE_LAST_HIGHLIGHT=""
_SHELLSENSE_SOCK="$HOME/.shellsense/daemon.sock"

# ── Ensure daemon is running ──
if ! echo '{"Ping"}' | nc -U "$_SHELLSENSE_SOCK" 2>/dev/null | grep -q Pong; then
    "$_SHELLSENSE_BIN" daemon &
fi

# ── preexec: capture command before it runs ──
_shellsense_preexec() { _SHELLSENSE_LAST_CMD="$1" }

# ── precmd: record command after it runs ──
_shellsense_precmd() {
    local exit_code=$?
    [[ -z "$_SHELLSENSE_LAST_CMD" ]] && return

    # Construct JSON payload for Add requests
    # Use careful quoting for zsh
    local cmd_esc="${_SHELLSENSE_LAST_CMD//\\/\\\\}"
    cmd_esc="${cmd_esc//\"/\\\"}"
    cmd_esc="${cmd_esc//$'\n'/\\n}"
    cmd_esc="${cmd_esc//$'\r'/\\r}"
    cmd_esc="${cmd_esc//$'\t'/\\t}"
    local dir_esc=${PWD//\"/\\\"}
    
    local ts
    ts=$(date +%s)
    local hour
    hour=$((10#$(date +%H)))
    
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

    # Send asynchronously without waiting for output
    echo "$json" | nc -U "$_SHELLSENSE_SOCK" &>/dev/null &

    _SHELLSENSE_CMD_2AGO="$_SHELLSENSE_CMD_1AGO"
    _SHELLSENSE_CMD_1AGO="$_SHELLSENSE_LAST_CMD"
    _SHELLSENSE_LAST_CMD=""
}

# ── Query suggestions (called on keystroke) ──
_shellsense_suggest() {
    local prefix="$BUFFER"
    if [[ ${#prefix} -lt 2 ]]; then
        _shellsense_clear
        return
    fi
    
    local pfx_esc="${prefix//\\/\\\\}"
    pfx_esc="${pfx_esc//\"/\\\"}"
    pfx_esc="${pfx_esc//$'\n'/\\n}"
    pfx_esc="${pfx_esc//$'\r'/\\r}"
    pfx_esc="${pfx_esc//$'\t'/\\t}"
    local dir_esc=${PWD//\"/\\\"}
    local json="{\"Suggest\":{\"prefix\":\"$pfx_esc\",\"dir\":\"$dir_esc\",\"count\":1,\"plain\":true"
    
    if [[ -n "$_SHELLSENSE_CMD_1AGO" ]]; then
        local p1="${_SHELLSENSE_CMD_1AGO//\\/\\\\}"; p1="${p1//\"/\\\"}"; p1="${p1//$'\n'/\\n}"; json+=",\"prev\":\"$p1\""
    fi
    if [[ -n "$_SHELLSENSE_CMD_2AGO" ]]; then
        local p2="${_SHELLSENSE_CMD_2AGO//\\/\\\\}"; p2="${p2//\"/\\\"}"; p2="${p2//$'\n'/\\n}"; json+=",\"prev2\":\"$p2\""
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
    
    if [[ -z "$raw" || "$raw" == '{"Error"'* ]]; then
        _shellsense_clear
        return
    fi

    # The daemon now returns plain text
    _SHELLSENSE_SUGGESTION="${raw%%$'\n'*}"
    _shellsense_show "$prefix"
}

# ── Clear all suggestion state ──
_shellsense_clear() {
    POSTDISPLAY=""
    _SHELLSENSE_SUGGESTION=""
    _SHELLSENSE_IS_CORRECTION=0
    
    if [[ -n "$_SHELLSENSE_LAST_HIGHLIGHT" ]]; then
        region_highlight=("${(@)region_highlight:#$_SHELLSENSE_LAST_HIGHLIGHT}")
        _SHELLSENSE_LAST_HIGHLIGHT=""
    fi
}

# ── Show ghost text for current suggestion ──
_shellsense_show() {
    local prefix="$1"
    if [[ -z "$_SHELLSENSE_SUGGESTION" ]]; then
        _shellsense_clear
        return
    fi
    local suggestion="$_SHELLSENSE_SUGGESTION"
    
    if [[ "$suggestion" = "$prefix" ]]; then
        _shellsense_clear
        return
    fi

    if [[ "$suggestion" = "$prefix"* ]]; then
        # Prefix match: show remainder as ghost text
        POSTDISPLAY="${suggestion:${#prefix}}"
        _SHELLSENSE_IS_CORRECTION=0
    else
        # Typo correction: show as a hint
        POSTDISPLAY="  > $suggestion"
        _SHELLSENSE_IS_CORRECTION=1
    fi

    # Apply syntax highlighting to the ghost text
    if [[ -n "$_SHELLSENSE_LAST_HIGHLIGHT" ]]; then
        region_highlight=("${(@)region_highlight:#$_SHELLSENSE_LAST_HIGHLIGHT}")
    fi
    
    if [[ -n "$POSTDISPLAY" ]]; then
        local color="fg=8" # dim gray
        if [[ "$_SHELLSENSE_IS_CORRECTION" = "1" ]]; then
            color="fg=3" # yellow for corrections
        fi
        
        _SHELLSENSE_LAST_HIGHLIGHT="$#BUFFER $(( $#BUFFER + $#POSTDISPLAY )) $color"
        region_highlight+=("$_SHELLSENSE_LAST_HIGHLIGHT")
    fi
}

# ── Accept full suggestion [Tab / Ctrl+F / →] ──
_shellsense_accept() {
    [[ -z "$POSTDISPLAY" ]] && return

    if [[ "$_SHELLSENSE_IS_CORRECTION" = "1" ]]; then
        BUFFER="${POSTDISPLAY#  > }"
    else
        BUFFER="$BUFFER$POSTDISPLAY"
    fi
    CURSOR=${#BUFFER}
    _shellsense_clear
}

# ── Accept one semantic word [Alt+F] ──
_shellsense_accept_word() {
    if [[ -n "$POSTDISPLAY" ]]; then
        if [[ "$_SHELLSENSE_IS_CORRECTION" = "1" ]]; then
            # Typo: accept the whole correction
            BUFFER="${POSTDISPLAY#  > }"
            CURSOR=${#BUFFER}
            _shellsense_clear
        else
            # Consume non-word characters + one word, OR just remaining non-word characters
            if [[ "$POSTDISPLAY" =~ "^([^a-zA-Z0-9_.-]*[a-zA-Z0-9_.-]+|[^a-zA-Z0-9_.-]+)" ]]; then
                local chunk="$MATCH"
                BUFFER="$BUFFER$chunk"
                POSTDISPLAY="${POSTDISPLAY#$chunk}"
                [[ -z "$POSTDISPLAY" ]] && _shellsense_clear
            else
                BUFFER="$BUFFER$POSTDISPLAY"
                _shellsense_clear
            fi
            CURSOR=${#BUFFER}
        fi
    else
        zle forward-word
    fi
}



# ── Dismiss [Escape] ──
_shellsense_dismiss() {
    _shellsense_clear
}


# ── Keystroke handler (self-insert override) ──
_shellsense_self_insert() {
    local last_char="${KEYS[-1]}"
    zle .self-insert

    # Fast path: consume matching ghost char without re-querying
    if [[ -n "$POSTDISPLAY" && "$_SHELLSENSE_IS_CORRECTION" != "1" ]]; then
        if [[ "$POSTDISPLAY" = "${last_char}"* ]]; then
            POSTDISPLAY="${POSTDISPLAY:1}"
            return
        fi
    fi

    _shellsense_suggest
}

# ── Backspace handler ──
_shellsense_backward_delete() {
    zle .backward-delete-char
    _shellsense_suggest
}

# ── Tab: accept or fallback to native completion ──
_shellsense_accept_or_complete() {
    if [[ -n "$POSTDISPLAY" ]]; then
        _shellsense_accept
    else
        zle expand-or-complete
    fi
}

# ── Ctrl+F / →: accept or fallback to native forward ──
_shellsense_forward_char() {
    if [[ -n "$POSTDISPLAY" ]]; then
        _shellsense_accept
    else
        zle forward-char
    fi
}

# ── Clean up leftover ghost text on new prompt ──
_shellsense_line_init() {
    _shellsense_clear
}

# ── Register ZLE widgets ──
zle -N self-insert _shellsense_self_insert
zle -N backward-delete-char _shellsense_backward_delete
zle -N _shellsense_accept
zle -N _shellsense_accept_word
zle -N _shellsense_dismiss
zle -N _shellsense_accept_or_complete
zle -N _shellsense_forward_char
zle -N zle-line-init _shellsense_line_init

# ── Keybindings (Mac & Unix Friendly) ──
#   Tab         accept or complete
#   → / Ctrl+E  accept or forward char
#   Ctrl+Space  accept one word
#   Escape      dismiss
bindkey '^I'       _shellsense_accept_or_complete
bindkey '^E'       _shellsense_forward_char
bindkey '^[[C'     _shellsense_forward_char
bindkey '^@'       _shellsense_accept_word
bindkey '^[[1;5C'  _shellsense_accept_word  # Ctrl+Right
bindkey '\e'       _shellsense_dismiss

# ── Hooks ──
autoload -Uz add-zsh-hook
add-zsh-hook preexec _shellsense_preexec
add-zsh-hook precmd _shellsense_precmd

# ── Startup ──
[[ -z "$_SHELLSENSE_LOADED" ]] && _SHELLSENSE_LOADED=1
