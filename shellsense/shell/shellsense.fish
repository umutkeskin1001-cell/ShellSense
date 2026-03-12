#!/usr/bin/env fish
# ShellSense — Offline Terminal AI Autocomplete (Fish Plugin)

set -g _SHELLSENSE_BIN (which shellsense 2>/dev/null)
if test -z "$_SHELLSENSE_BIN"
    if test -x "$HOME/.shellsense/bin/shellsense"
        set _SHELLSENSE_BIN "$HOME/.shellsense/bin/shellsense"
    else if test -x "$HOME/.cargo/bin/shellsense"
        set _SHELLSENSE_BIN "$HOME/.cargo/bin/shellsense"
    else
        echo "[shellsense] Binary not found. Run: cargo install --path /path/to/shellsense"
        return 1
    end
end

set -g _SHELLSENSE_SESSION $fish_pid
set -g _SHELLSENSE_CMD_1AGO ""
set -g _SHELLSENSE_CMD_2AGO ""
set -g _SHELLSENSE_SOCK "$HOME/.shellsense/daemon.sock"

# Ensure daemon is running
if not echo '{"Ping"}' | nc -U "$_SHELLSENSE_SOCK" 2>/dev/null | grep -q Pong
    eval "$_SHELLSENSE_BIN daemon &"
end

# Hook on command event (runs before prompt)
function _shellsense_postexec --on-event fish_postexec
    set -l exit_code $status
    set -l last_cmd $argv[1]
    
    if test -z "$last_cmd"
        return
    end

    # Escape quotes and control characters for JSON
    set -l cmd_esc (string replace -a '\\' '\\\\' "$last_cmd" | string replace -a '"' '\\"' | string replace -a \n '\\n' | string replace -a \r '\\r' | string replace -a \t '\\t')
    set -l dir_esc (string replace -a '\\' '\\\\' "$PWD" | string replace -a '"' '\\"')
    set -l ts (date +%s)
    set -l hr (math (date +%H))
    
    set -l json "{\"Add\":{\"cmd\":\"$cmd_esc\",\"dir\":\"$dir_esc\",\"exit\":$exit_code,\"session\":\"$_SHELLSENSE_SESSION\",\"timestamp\":$ts,\"hour\":$hr"
    
    if test -d .git; or git rev-parse --git-dir >/dev/null 2>&1
        set -l b (git branch --show-current 2>/dev/null)
        if test -n "$b"
            set json "$json,\"git\":\"$b\""
        end
    end
    
    if test -n "$_SHELLSENSE_CMD_1AGO"
        set -l prev_esc (string replace -a '\\' '\\\\' "$_SHELLSENSE_CMD_1AGO" | string replace -a '"' '\\"' | string replace -a \n '\\n')
        set json "$json,\"prev\":\"$prev_esc\""
    end
    
    if test -n "$_SHELLSENSE_CMD_2AGO"
        set -l prev2_esc (string replace -a '\\' '\\\\' "$_SHELLSENSE_CMD_2AGO" | string replace -a '"' '\\"' | string replace -a \n '\\n')
        set json "$json,\"prev2\":\"$prev2_esc\""
    end
    
    set json "$json}}"
    
    echo "$json" | nc -U "$_SHELLSENSE_SOCK" >/dev/null 2>&1 &

    set -g _SHELLSENSE_CMD_2AGO "$_SHELLSENSE_CMD_1AGO"
    set -g _SHELLSENSE_CMD_1AGO "$last_cmd"
end

# In Fish, we integrate into the commandline function for suggestions.
function _shellsense_fish_suggest
    set -l prefix (commandline -b)
    if test (string length "$prefix") -lt 2
        return
    end

    set -l pfx_esc (string replace -a '\\' '\\\\' "$prefix" | string replace -a '"' '\\"' | string replace -a \n '\\n' | string replace -a \r '\\r' | string replace -a \t '\\t')
    set -l dir_esc (string replace -a '\\' '\\\\' "$PWD" | string replace -a '"' '\\"')
    set -l json "{\"Suggest\":{\"prefix\":\"$pfx_esc\",\"dir\":\"$dir_esc\",\"count\":1,\"plain\":true"
    
    if test -n "$_SHELLSENSE_CMD_1AGO"
        set -l prev_esc (string replace -a '\\' '\\\\' "$_SHELLSENSE_CMD_1AGO" | string replace -a '"' '\\"' | string replace -a \n '\\n')
        set json "$json,\"prev\":\"$prev_esc\""
    end
    
    set -l has_env 0
    if test -n "$VIRTUAL_ENV"; or test -n "$KUBECONFIG"; or test -n "$AWS_PROFILE"
        set json "$json,\"env\":["
        if test -n "$VIRTUAL_ENV"
            set json "$json\"VIRTUAL_ENV\""
            set has_env 1
        end
        if test -n "$KUBECONFIG"
            if test $has_env -eq 1; set json "$json,"; end
            set json "$json\"KUBECONFIG\""
            set has_env 1
        end
        if test -n "$AWS_PROFILE"
            if test $has_env -eq 1; set json "$json,"; end
            set json "$json\"AWS_PROFILE\""
        end
        set json "$json]"
    end
    
    set json "$json}}"

    if not type -q nc
        return
    end

    set -l raw (echo "$json" | nc -U "$_SHELLSENSE_SOCK" 2>/dev/null)
    if test -z "$raw"
        return
    end
    if string match -q '*"Error"*' "$raw"
        return
    end

    # Read first line of plain text
    set -l arr (string split "\n" "$raw")
    if test -n "$arr[1]"
        set -l suggestion "$arr[1]"

        if test "$suggestion" != "$prefix"
            commandline -r "$suggestion"
            commandline -C (string length "$suggestion")
        end
    end
end

bind \ce _shellsense_fish_suggest
