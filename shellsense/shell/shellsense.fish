#!/usr/bin/env fish
# ShellSense — Offline terminal autocomplete for Fish

if not set -q _SHELLSENSE_LOADED
    set -g _SHELLSENSE_LOADED 1

    set -g _SHELLSENSE_BIN (command -v shellsense 2>/dev/null)
    if test -z "$_SHELLSENSE_BIN"
        if test -x "$HOME/.shellsense/bin/shellsense"
            set _SHELLSENSE_BIN "$HOME/.shellsense/bin/shellsense"
        else if test -x "$HOME/.cargo/bin/shellsense"
            set _SHELLSENSE_BIN "$HOME/.cargo/bin/shellsense"
        else
            echo "[shellsense] Binary not found. Run: cargo install --path /path/to/shellsense"
        end
    end

    if test -n "$_SHELLSENSE_BIN"
        set -g _SHELLSENSE_CMD_1AGO ""
        set -g _SHELLSENSE_CMD_2AGO ""

        if set -q SHELLSENSE_DATA_DIR
            set -g _SHELLSENSE_SOCK "$SHELLSENSE_DATA_DIR/daemon.sock"
        else
            set -g _SHELLSENSE_SOCK "$HOME/.shellsense/daemon.sock"
        end

        function _shellsense_json_escape
            set -l escaped (string replace -a '\\' '\\\\' -- "$argv[1]")
            set escaped (string replace -a '"' '\\"' -- "$escaped")
            set escaped (string replace -a \n '\\n' -- "$escaped")
            set escaped (string replace -a \r '\\r' -- "$escaped")
            set escaped (string replace -a \t '\\t' -- "$escaped")
            printf '%s' "$escaped"
        end

        function _shellsense_ensure_daemon
            $_SHELLSENSE_BIN ping >/dev/null 2>&1
            if test $status -ne 0
                $_SHELLSENSE_BIN daemon >/dev/null 2>&1 &
            end
        end

        function _shellsense_postexec --on-event fish_postexec
            set -l last_cmd $argv[1]
            if test -z "$last_cmd"
                return
            end
            if not type -q nc
                return
            end

            _shellsense_ensure_daemon

            set -l cmd_esc (_shellsense_json_escape "$last_cmd")
            set -l dir_esc (_shellsense_json_escape "$PWD")
            set -l json "{\"Add\":{\"cmd\":\"$cmd_esc\",\"dir\":\"$dir_esc\""

            if test -n "$_SHELLSENSE_CMD_1AGO"
                set -l prev_esc (_shellsense_json_escape "$_SHELLSENSE_CMD_1AGO")
                set json "$json,\"prev\":\"$prev_esc\""
            end

            if test -n "$_SHELLSENSE_CMD_2AGO"
                set -l prev2_esc (_shellsense_json_escape "$_SHELLSENSE_CMD_2AGO")
                set json "$json,\"prev2\":\"$prev2_esc\""
            end

            set json "$json}}"
            printf '%s\n' "$json" | nc -U "$_SHELLSENSE_SOCK" >/dev/null 2>&1 &

            set -g _SHELLSENSE_CMD_2AGO "$_SHELLSENSE_CMD_1AGO"
            set -g _SHELLSENSE_CMD_1AGO "$last_cmd"
        end

        function _shellsense_fish_suggest
            set -l prefix (commandline -b)
            if test (string length "$prefix") -lt 2
                return
            end
            if not type -q nc
                return
            end

            set -l pfx_esc (_shellsense_json_escape "$prefix")
            set -l dir_esc (_shellsense_json_escape "$PWD")
            set -l json "{\"Suggest\":{\"prefix\":\"$pfx_esc\",\"dir\":\"$dir_esc\",\"count\":1,\"plain\":true"

            if test -n "$_SHELLSENSE_CMD_1AGO"
                set -l prev_esc (_shellsense_json_escape "$_SHELLSENSE_CMD_1AGO")
                set json "$json,\"prev\":\"$prev_esc\""
            end

            if test -n "$_SHELLSENSE_CMD_2AGO"
                set -l prev2_esc (_shellsense_json_escape "$_SHELLSENSE_CMD_2AGO")
                set json "$json,\"prev2\":\"$prev2_esc\""
            end

            set -l has_env 0
            if test -n "$VIRTUAL_ENV"; or test -n "$KUBECONFIG"; or test -n "$AWS_PROFILE"
                set json "$json,\"env\":["
                if test -n "$VIRTUAL_ENV"
                    set json "$json\"VIRTUAL_ENV\""
                    set has_env 1
                end
                if test -n "$KUBECONFIG"
                    if test $has_env -eq 1
                        set json "$json,"
                    end
                    set json "$json\"KUBECONFIG\""
                    set has_env 1
                end
                if test -n "$AWS_PROFILE"
                    if test $has_env -eq 1
                        set json "$json,"
                    end
                    set json "$json\"AWS_PROFILE\""
                end
                set json "$json]"
            end

            set json "$json}}"
            set -l raw (printf '%s\n' "$json" | nc -U "$_SHELLSENSE_SOCK" 2>/dev/null)
            if test -z "$raw"
                _shellsense_ensure_daemon
                set raw (printf '%s\n' "$json" | nc -U "$_SHELLSENSE_SOCK" 2>/dev/null)
            end

            if test -z "$raw"
                return
            end
            if string match -q '*"Error"*' "$raw"
                return
            end

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
        _shellsense_ensure_daemon
    end
end
