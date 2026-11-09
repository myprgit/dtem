# dtem

GPU-accelerated terminal emulator. It runs as a normal window or as a dropdown, and keeps its whole session across restarts and reboots.

- Renders through **wgpu** (Vulkan/GL). Terminal emulation comes from `alacritty_terminal`.
- Supports tabs and split panes (a split tree like kitty's "splits" layout).
- **Session persistence**, saved automatically every 5 s, on exit and on SIGTERM/SIGHUP:
  - all windows and tabs come back
  - each pane reopens in its last **cwd**
  - **scrollback** is restored, including the primary screen behind vim/htop
  - **split layout** and ratios, plus window size, position and monitor
  - running programs are **not** restarted: a pane that was in nvim comes back as a shell in the same directory
- Dropdown mode is a separate process with its own session. It remembers **the monitor you last moved it to**.
- Ships kitty-like shortcuts, all of them configurable.
- Asks for confirmation before closing a tab, pane or window. You can turn this off in the config.

## Usage

```
dtem                          normal window (restores the last session)
dtem --dropdown [--hidden]    dropdown terminal
dtem --toggle                 show/hide dropdown (bind this to a key)
dtem --session FILE.json      open a layout file
dtem --no-restore             start fresh
dtem --install-gnome-shortcut register the dropdown hotkey in GNOME
dtem --quit [--dropdown]      quit the running instance (session saved)
```

Install: `./install.sh --shortcut`

## Dropdown and monitors

- Under X11, the hotkey (default `F12`) is grabbed globally. Under GNOME, both X11 and Wayland, run `dtem --install-gnome-shortcut` instead. It registers `dtem --toggle` as a custom shortcut.
- The dropdown always appears on the monitor you last moved it to. To move it, either:
  - use `kitty_mod+alt+m` (next monitor), or
  - use GNOME's `Super+Shift+←/→`.
- The chosen monitor is stored in the session, so it survives reboot. If that monitor is unplugged, the dropdown falls back to the primary monitor.
- When the hotkey is pressed while the dropdown is visible but unfocused, it is focused. If it is already focused, it is hidden.

## Wayland

- Normal windows run natively on Wayland. Size and maximized state are restored. Wayland does not let apps set their own window position; set `force_x11 = true` to get position restore through XWayland.
- On Wayland the dropdown runs through XWayland. GNOME has no layer-shell, so absolute positioning and per-monitor placement only work this way. Guake does the same.
- The clipboard uses the native Wayland or X11 clipboard, including the primary selection.

## Shortcuts (`kitty_mod` = ctrl+shift)

| Action | Key |
|---|---|
| new tab (inherits cwd) / close tab | `kitty_mod+t` / `kitty_mod+q` |
| next / previous tab | `kitty_mod+→` `ctrl+tab` / `kitty_mod+←` `ctrl+shift+tab` |
| go to tab N | `alt+1..9` |
| move tab | `kitty_mod+.` / `kitty_mod+,` |
| rename tab | `kitty_mod+alt+t` |
| new split (along the longer side) | `kitty_mod+enter` |
| hsplit / vsplit / rotate split | `F5` / `F6` / `F7` |
| close pane | `kitty_mod+w` |
| next / previous pane | `kitty_mod+]` / `kitty_mod+[` |
| pane N | `kitty_mod+1..9` |
| focus neighbour | `kitty_mod+alt+h/j/k/l` |
| resize pane | `kitty_mod+alt+arrows` |
| move pane | `kitty_mod+f` / `kitty_mod+b` |
| zoom pane | `kitty_mod+l` |
| copy / paste / paste selection | `kitty_mod+c` / `kitty_mod+v` / `shift+insert` |
| scroll | `kitty_mod+↑/↓/k/j`, `shift+pgup/pgdn`, `kitty_mod+home/end` |
| clear scrollback | `kitty_mod+delete` |
| font size | `kitty_mod+=` / `kitty_mod+-` / `kitty_mod+backspace` |
| new OS window | `kitty_mod+n` |
| fullscreen | `kitty_mod+f11` |
| export layout to `~/.config/dtem/sessions/` | `kitty_mod+alt+s` |
| reload config | `kitty_mod+f5` |
| dropdown → next monitor | `kitty_mod+alt+m` |

Key bindings match on the physical key position as a fallback, so they also work on Turkish Q and F layouts.

## Session / layout files

These are plain JSON, in the same format as the autosave. Example:

```json
{"windows":[{"tabs":[
  {"title":"api","layout":{"Split":{"vertical":true,"ratio":0.6,
    "a":{"Leaf":{"cwd":"~/proj/api","command":"nvim ."}},
    "b":{"Leaf":{"cwd":"~/proj/api","command":"ssh devbox"}}}}},
  {"title":"logs","layout":{"Leaf":{"cwd":"/var/log"}}}
]}]}
```

`command` runs once and then drops you into a shell in that pane. Autosaved sessions never re-run commands.

Files:
- config: `~/.config/dtem/dtem.toml` (see `dtem.example.toml`)
- state: `~/.local/state/dtem/`
