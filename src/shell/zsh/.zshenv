# dtem shell integration for zsh: restore the real ZDOTDIR, load the user's .zshenv, add hooks
if [[ -n "${DTEM_ORIG_ZDOTDIR+x}" ]]; then
  if [[ -n "$DTEM_ORIG_ZDOTDIR" ]]; then ZDOTDIR="$DTEM_ORIG_ZDOTDIR"; else unset ZDOTDIR; fi
  unset DTEM_ORIG_ZDOTDIR
fi
[[ -f "${ZDOTDIR:-$HOME}/.zshenv" ]] && source "${ZDOTDIR:-$HOME}/.zshenv"
if [[ -o interactive && -z "$__dtem_loaded" ]]; then
  __dtem_loaded=1
  __dtem_running=
  autoload -Uz add-zsh-hook
  __dtem_precmd() {
    local ec=$?
    # per-tab history (after .zshrc, which may have set HISTFILE / SHARE_HISTORY)
    if [[ -n "$DTEM_HISTFILE" && "$HISTFILE" != "$DTEM_HISTFILE" ]]; then
      unsetopt share_history inc_append_history_time 2>/dev/null
      setopt inc_append_history
      fc -p "$DTEM_HISTFILE" "${HISTSIZE:-1000}" "${SAVEHIST:-1000}"
    fi
    [[ -n $__dtem_running ]] && printf '\e]133;D;%s\a' $ec
    __dtem_running=
    printf '\e]7;file://%s%s\a\e]133;A\a' "$HOST" "$PWD"
  }
  __dtem_preexec() {
    __dtem_running=1
    printf '\e]133;C\a'
  }
  add-zsh-hook precmd __dtem_precmd
  add-zsh-hook preexec __dtem_preexec
fi
