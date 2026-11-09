# dtem shell integration for bash (loaded via --rcfile; your own startup files are sourced first)
if [ -n "$DTEM_BASH_LOGIN" ]; then
  unset DTEM_BASH_LOGIN
  [ -r /etc/profile ] && . /etc/profile
  for __dtem_f in ~/.bash_profile ~/.bash_login ~/.profile; do
    if [ -r "$__dtem_f" ]; then . "$__dtem_f"; break; fi
  done
  unset __dtem_f
else
  [ -r /etc/bash.bashrc ] && . /etc/bash.bashrc
  [ -r ~/.bashrc ] && . ~/.bashrc
fi

if [[ $- == *i* && -z "$__dtem_loaded" ]]; then
  __dtem_loaded=1
  __dtem_running=
  __dtem_ready=
  __dtem_precmd() {
    local ec=$?
    [ -n "$__dtem_running" ] && printf '\e]133;D;%s\a' "$ec"
    __dtem_running=
    return $ec
  }
  __dtem_prompt() {
    local ec=$?
    printf '\e]7;file://%s%s\a\e]133;A\a' "$HOSTNAME" "$PWD"
    __dtem_ready=1
    return $ec
  }
  __dtem_start() {
    __dtem_ready=
    __dtem_running=1
    printf '\e]133;C\a'
  }
  __dtem_preexec() {
    [ -n "$COMP_LINE" ] && return
    [ -z "$__dtem_ready" ] && return
    case "$BASH_COMMAND" in __dtem_*) return ;; esac
    __dtem_start
  }
  if [[ "$(declare -p PROMPT_COMMAND 2>/dev/null)" == "declare -a"* ]]; then
    PROMPT_COMMAND=(__dtem_precmd "${PROMPT_COMMAND[@]}" __dtem_prompt)
  else
    PROMPT_COMMAND="__dtem_precmd${PROMPT_COMMAND:+; $PROMPT_COMMAND}; __dtem_prompt"
  fi
  if [ -n "${bash_preexec_imported:-}" ] || declare -F __bp_install >/dev/null 2>&1; then
    preexec_functions+=(__dtem_start)
  else
    trap '__dtem_preexec' DEBUG
  fi
fi
