#!/usr/bin/env zsh
# organism-shell-hook.zsh
#
# Wires zsh terminal events into a running organism daemon.
#
# Install:
#   echo 'source /path/to/scripts/organism-shell-hook.zsh' >> ~/.zshrc
#
# Behavior:
#   * preexec captures the command line and start timestamp.
#   * precmd computes exit code + duration, then fires
#     `organism-cli emit-terminal ...` in the background (`&!`) so the
#     prompt is never blocked.
#   * If `organism-cli` is not on PATH, hooks no-op silently.
#   * All stdout/stderr from the CLI is sent to /dev/null; if the daemon
#     is down, the user is never spammed with errors.

# Guard against double-sourcing.
if [[ -n "${__ORGANISM_HOOK_LOADED:-}" ]]; then
  return 0
fi
typeset -g __ORGANISM_HOOK_LOADED=1

typeset -g __ORGANISM_LAST_CMD=""
typeset -g __ORGANISM_LAST_START=0

__organism_now_ms() {
  if zmodload -e zsh/datetime 2>/dev/null || zmodload zsh/datetime 2>/dev/null; then
    local _er=${EPOCHREALTIME}
    local _sec=${_er%.*} _frac=${_er#*.}
    _frac=${_frac}000
    print -r -- "$(( _sec * 1000 + 10#${_frac[1,3]} ))"
  else
    print -r -- "$(( $(date +%s) * 1000 ))"
  fi
}

__organism_preexec() {
  __ORGANISM_LAST_CMD="$1"
  __ORGANISM_LAST_START="$(__organism_now_ms)"
}

__organism_precmd() {
  # CRITICAL: capture $? FIRST -- any other command will clobber it.
  local ec=$?

  if [[ -z "${__ORGANISM_LAST_CMD}" ]]; then
    return 0
  fi

  if ! command -v organism-cli >/dev/null 2>&1; then
    __ORGANISM_LAST_CMD=""
    __ORGANISM_LAST_START=0
    return 0
  fi

  local now duration cmd cwd
  now="$(__organism_now_ms)"
  duration=$(( now - __ORGANISM_LAST_START ))
  if (( duration < 0 )); then duration=0; fi
  cmd="${__ORGANISM_LAST_CMD}"
  cwd="${PWD}"

  __ORGANISM_LAST_CMD=""
  __ORGANISM_LAST_START=0

  # Fire-and-forget. `&!` disowns immediately so the prompt never blocks.
  { organism-cli emit-terminal "$cmd" \
      --exit-code "$ec" \
      --cwd "$cwd" \
      --duration-ms "$duration" >/dev/null 2>&1 } &!
}

if autoload -Uz add-zsh-hook 2>/dev/null; then
  add-zsh-hook preexec __organism_preexec
  add-zsh-hook precmd  __organism_precmd
else
  typeset -ga preexec_functions precmd_functions
  preexec_functions+=(__organism_preexec)
  precmd_functions+=(__organism_precmd)
fi
