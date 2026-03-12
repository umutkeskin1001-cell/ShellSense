#!/usr/bin/env bash
# ShellSense — Offline terminal autocomplete for Bash

if [[ -z "${_SHELLSENSE_LOADED:-}" ]]; then
    _SHELLSENSE_LOADED=1

    if command -v shellsense &>/dev/null; then
        _SHELLSENSE_BIN="shellsense"
    elif [[ -x "$HOME/.shellsense/bin/shellsense" ]]; then
        _SHELLSENSE_BIN="$HOME/.shellsense/bin/shellsense"
    elif [[ -x "$HOME/.cargo/bin/shellsense" ]]; then
        _SHELLSENSE_BIN="$HOME/.cargo/bin/shellsense"
    else
        echo "[shellsense] Binary not found."
    fi

    if [[ -n "${_SHELLSENSE_BIN:-}" ]]; then
        _SHELLSENSE_CMD_1AGO=""
        _SHELLSENSE_CMD_2AGO=""
        _SHELLSENSE_LAST_CMD=""

        if [[ -n "${SHELLSENSE_DATA_DIR:-}" ]]; then
            _SHELLSENSE_SOCK="$SHELLSENSE_DATA_DIR/daemon.sock"
        else
            _SHELLSENSE_SOCK="$HOME/.shellsense/daemon.sock"
        fi

        _shellsense_json_escape() {
            local value="$1"
            value="${value//\\/\\\\}"
            value="${value//\"/\\\"}"
            value="${value//$'\n'/\\n}"
            value="${value//$'\r'/\\r}"
            value="${value//$'\t'/\\t}"
            printf '%s' "$value"
        }

        _shellsense_ensure_daemon() {
            "$_SHELLSENSE_BIN" ping >/dev/null 2>&1 && return 0
            "$_SHELLSENSE_BIN" daemon >/dev/null 2>&1 &
        }

        _shellsense_prompt_command() {
            local last_hist
            last_hist=$(history 1 | sed -e "s/^[ ]*[0-9]*[ ]*//")
            _SHELLSENSE_LAST_CMD="$last_hist"

            [[ -z "$_SHELLSENSE_LAST_CMD" ]] && return
            [[ "$_SHELLSENSE_LAST_CMD" == "$_SHELLSENSE_CMD_1AGO" ]] && return
            command -v nc >/dev/null 2>&1 || return

            _shellsense_ensure_daemon

            local cmd_esc
            cmd_esc="$(_shellsense_json_escape "$_SHELLSENSE_LAST_CMD")"
            local dir_esc
            dir_esc="$(_shellsense_json_escape "$PWD")"
            local json="{\"Add\":{\"cmd\":\"$cmd_esc\",\"dir\":\"$dir_esc\""

            if [[ -n "$_SHELLSENSE_CMD_1AGO" ]]; then
                local prev_esc
                prev_esc="$(_shellsense_json_escape "$_SHELLSENSE_CMD_1AGO")"
                json+=",\"prev\":\"$prev_esc\""
            fi
            if [[ -n "$_SHELLSENSE_CMD_2AGO" ]]; then
                local prev2_esc
                prev2_esc="$(_shellsense_json_escape "$_SHELLSENSE_CMD_2AGO")"
                json+=",\"prev2\":\"$prev2_esc\""
            fi

            json+="}}"
            printf '%s\n' "$json" | nc -U "$_SHELLSENSE_SOCK" >/dev/null 2>&1 &

            _SHELLSENSE_CMD_2AGO="$_SHELLSENSE_CMD_1AGO"
            _SHELLSENSE_CMD_1AGO="$_SHELLSENSE_LAST_CMD"
        }

        _shellsense_suggest_bash() {
            local prefix="${READLINE_LINE:0:$READLINE_POINT}"
            if [[ ${#prefix} -lt 2 ]]; then
                return
            fi

            command -v nc >/dev/null 2>&1 || return

            local pfx_esc
            pfx_esc="$(_shellsense_json_escape "$prefix")"
            local dir_esc
            dir_esc="$(_shellsense_json_escape "$PWD")"
            local json="{\"Suggest\":{\"prefix\":\"$pfx_esc\",\"dir\":\"$dir_esc\",\"count\":1,\"plain\":true"

            if [[ -n "$_SHELLSENSE_CMD_1AGO" ]]; then
                local prev_esc
                prev_esc="$(_shellsense_json_escape "$_SHELLSENSE_CMD_1AGO")"
                json+=",\"prev\":\"$prev_esc\""
            fi
            if [[ -n "$_SHELLSENSE_CMD_2AGO" ]]; then
                local prev2_esc
                prev2_esc="$(_shellsense_json_escape "$_SHELLSENSE_CMD_2AGO")"
                json+=",\"prev2\":\"$prev2_esc\""
            fi

            local has_env=0
            if [[ -n "$VIRTUAL_ENV" || -n "$KUBECONFIG" || -n "$AWS_PROFILE" ]]; then
                json+=",\"env\":["
                [[ -n "$VIRTUAL_ENV" ]] && json+="\"VIRTUAL_ENV\"" && has_env=1
                if [[ -n "$KUBECONFIG" ]]; then
                    [[ $has_env -eq 1 ]] && json+=","
                    json+="\"KUBECONFIG\""
                    has_env=1
                fi
                if [[ -n "$AWS_PROFILE" ]]; then
                    [[ $has_env -eq 1 ]] && json+=","
                    json+="\"AWS_PROFILE\""
                fi
                json+="]"
            fi
            json+="}}"

            local raw
            raw=$(printf '%s\n' "$json" | nc -U "$_SHELLSENSE_SOCK" 2>/dev/null)
            if [[ -z "$raw" ]]; then
                _shellsense_ensure_daemon
                raw=$(printf '%s\n' "$json" | nc -U "$_SHELLSENSE_SOCK" 2>/dev/null)
            fi

            [[ -z "$raw" || "$raw" == '{"Error"'* ]] && return

            local suggestion
            IFS=$'\n' read -r suggestion <<< "$raw"
            if [[ -n "$suggestion" && "$suggestion" != "$prefix" ]]; then
                READLINE_LINE="$suggestion"
                READLINE_POINT=${#READLINE_LINE}
            fi
        }

        bind -x '"\C-e": _shellsense_suggest_bash'

        if [[ -z "${PROMPT_COMMAND:-}" ]]; then
            PROMPT_COMMAND="_shellsense_prompt_command"
        elif [[ ";${PROMPT_COMMAND};" != *";_shellsense_prompt_command;"* ]]; then
            PROMPT_COMMAND="${PROMPT_COMMAND%;}; _shellsense_prompt_command"
        fi

        _shellsense_ensure_daemon
    fi
fi
