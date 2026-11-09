# dtem shell integration for fish
if status is-interactive; and not set -q __dtem_loaded
  set -g __dtem_loaded 1
  function __dtem_prompt --on-event fish_prompt
    set -l ec $status
    if set -q __dtem_running
      printf '\e]133;D;%s\a' $ec
      set -e __dtem_running
    end
    printf '\e]7;file://%s%s\a\e]133;A\a' (hostname) "$PWD"
  end
  function __dtem_preexec --on-event fish_preexec
    set -g __dtem_running 1
    printf '\e]133;C\a'
  end
end
