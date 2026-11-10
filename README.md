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
- **Text rendering:** programming **ligatures** (via swash shaping) and **color emoji**.
- **Links:** Ctrl+click opens URLs and OSC 8 hyperlinks. They are underlined while Ctrl is held.
- **Input:** **IME** support (ibus/fcitx) and the **kitty keyboard protocol**, which nvim and similar apps can turn on.
- **Transparency:** background opacity is configurable (needs a compositor).
- **`dtem --remote`:** SSH with clipboard and file sync (see below).

## Coming from tilda

```
dtem --import-tilda              # reads ~/.config/tilda/config_0 (or pass a file)
```

The importer converts every tilda option: hotkey, size/position/centering, monitor, slide animation, auto-hide, the unfocused-hotkey behaviour, always-on-top, sticky, taskbar, transparency, colors and palette, cursor, bold-is-bright, bell, scrollback, word chars, backspace/delete codes, command/login shell/working dir, title options, tab bar options, link matching, confirmations, and all key bindings. Your previous config is backed up first. A few options have no counterpart in dtem (tooltips, notebook border, set-as-desktop); the importer lists them.

## Notifications and tab markers

- Tabs you aren't looking at get a marker: **●** new output, **🔔** bell, **✗** a command failed.
- Desktop notifications (click one to jump to the tab) are sent when:
  - a command that ran ≥ 10 s finishes while you aren't looking (`notify_command_seconds`),
  - a bell rings in a background tab — e.g. an agent waiting for input (`notify_on_bell`),
  - a program sends OSC 9 / OSC 777 (`notify_programs`).
- When a shell exits by itself (`exit`, Ctrl+D), dtem asks: Enter closes the pane, `r` restarts the shell and Esc keeps the pane open (`confirm_close.on_exit`).

## Shell integration

bash, zsh and fish are started with a small integration script. Your own `.bashrc`/`.zshrc` are loaded as usual; dtem only adds prompt marks (OSC 133) and cwd reports (OSC 7). It's turned off with `shell_integration = false`. With it:

| | |
|---|---|
| `kitty_mod+z` / `kitty_mod+x` | jump to the previous / next prompt |
| `kitty_mod+g` | copy the last command's output |
| long-running command notifications | see above |
| ssh tabs | the remote directory is remembered; after a restart, reconnecting opens there |
| separate history | every tab has its own command history (Up arrow), kept across restarts; new tabs start from `~/.bash_history` (`separate_history`) |
| suggestions while typing | bash: install [ble.sh](https://github.com/akinomyoga/ble.sh) into `~/.local/share/blesh` and dtem loads it (dim suggestion, → accepts, Tab shows candidates, syntax colors); `autosuggestions = false` turns it off |

On hosts opened with `dtem --remote`, the integration, `dtem-copy` and `dtem-icat` are installed into `~/.local` automatically.

## Keyboard hints

`kitty_mod+e` puts letters on every link, path, IP address, git hash and e-mail address on the screen. Press a letter to open the link (or copy anything else). Press Shift+letter to type the item into the terminal instead.

## Images

dtem implements the kitty graphics protocol, so images work with `dtem --icat file.png`, with `dtem-icat` on remote hosts, and with tools that support the protocol (yazi, chafa, timg, …). Images scroll with the text and stay in the scrollback.

## Second dropdown

```toml
[dropdowns.2]
hotkey = "shift+F1"
height = 0.6
color_scheme = "dracula"
```

Each dropdown has its own tabs, session, monitor memory and settings; they appear on the settings page as **Dropdown** and **Dropdown 2**. `dtem --toggle=2` toggles it, and `dtem --install-gnome-shortcut` registers the hotkeys of all dropdowns.

## Right-click menu

Right-click on a terminal (or the tab bar) opens a menu like tilda's: New Tab, Close Tab, Copy, Paste, Open/Copy Link (when clicking a link), splits, Toggle Fullscreen / Transparency / Searchbar, Preferences…, Keyboard Shortcuts…, and Quit. Each item shows its current shortcut. If a program uses the mouse (vim, htop), use **Shift+right-click**.

## Settings page

Open it with `ctrl+,` or `kitty_mod+F2`, or with right-click → Preferences… (it opens in a new tab). You can also run `dtem --settings`. The **Keys** section is the shortcut editor (right-click → Keyboard Shortcuts…, or `dtem --settings=keys`): select an action and press Enter, then press the new key combination. `a` adds a second combination, `e` lets you type combinations, `x` unbinds the action and `r` restores its default. If a combination is already used by another action, it is moved to the new one and the editor reports this. All options are grouped there: appearance, colors, dropdown, behavior, shell & titles, session and key bindings. Changes are saved to `dtem.toml` with comments preserved, and running instances apply them live (config hot-reload).

## Usage

```
dtem                          normal window (restores the last session)
dtem --dropdown [--hidden]    dropdown terminal
dtem --toggle                 show/hide dropdown (bind this to a key)
dtem --session FILE.json      open a layout file
dtem --remote [ssh args] HOST ssh in a new tab with clipboard/file sync
dtem --no-restore             start fresh
dtem --install-gnome-shortcut register the dropdown hotkey in GNOME
dtem --quit [--dropdown] [--no-save]  quit (--no-save: forget the session)
dtem --settings               settings page
dtem --import-tilda [FILE]    import a tilda config
```

Install: `./install.sh --shortcut`

## Remote (SSH) and clipboard sync

```
dtem --remote user@host
dtem --remote -p 2222 -i ~/.ssh/key user@host     # any ssh options
```

Typed inside a dtem terminal, it runs ssh **right in that pane** (like `kitten ssh`); use `--new-tab` to open it in a new tab instead. Started from elsewhere, it opens a tab in the running dtem. The session is multiplexed through an SSH ControlMaster, so file transfers reuse the same authenticated connection and never ask for a password again.

| What | How |
|---|---|
| Local image → remote | Copy an image (e.g. a screenshot) and press `kitty_mod+v` in the ssh pane. It works like herdr: the image is staged in a private temp dir on the server (`/tmp/dtem-clipboard-<uid>/`, mode 0600, removed after 24 h) and its bare path is pasted. Agents such as Claude Code attach it as an image. |
| Local files → remote | Copy files in the file manager and paste, or drag & drop them onto the pane. They are uploaded and their paths are typed. |
| Remote text → local clipboard | `echo hi \| dtem-copy`, or any program using OSC 52 (nvim, tmux with `set-clipboard on`). |
| Remote image/file → local clipboard | `dtem-copy shot.png`. Images go on your clipboard as images. Other files are saved to `~/Downloads/dtem/` and copied as files. |
| Remote programs reading your clipboard (OSC 52) | Asks first: allow once / always / deny (`clipboard_read`). |

- **Helper install:** `dtem-copy` is installed into `~/.local/bin` on the server on first connect. Turn this off with `remote_install_helpers = false`.
- **Manually started ssh:** if you start `ssh host` yourself inside a local pane, image/file paste still works through a separate `ssh -o BatchMode=yes` connection. That connection needs key or agent authentication.
- **Disconnect:** the pane offers to reconnect.
- **After restore:** remote tabs come back with their scrollback and ask before reconnecting. Programs are never restarted.
- **Local panes:** pasting an image stages it the same way in a local temp dir and pastes the path.

## Dropdown and monitors

- Under X11, the hotkey (default `F12`; several keys allowed, e.g. `["F1", "ctrl+alt+t"]`) is grabbed globally. Keys that already have a GNOME shortcut are not grabbed a second time. Under GNOME, both X11 and Wayland, run `dtem --install-gnome-shortcut` instead. It registers `dtem --toggle` as a custom shortcut.
- The dropdown always appears on the monitor you last moved it to. To move it, either:
  - use `kitty_mod+alt+m` (next monitor), or
  - use GNOME's `Super+Shift+←/→`.
- The chosen monitor, and whether the dropdown was fullscreen when last hidden, are stored in the session, so they survive reboot. If that monitor is unplugged, the dropdown falls back to the primary monitor.
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
| move pane | `kitty_mod+alt+f` / `kitty_mod+alt+b` |
| zoom pane | `kitty_mod+l` |
| copy / paste / paste selection | `kitty_mod+c` / `kitty_mod+v` / `shift+insert` |
| open URL | `ctrl+click` |
| scroll | `kitty_mod+↑/↓/k/j`, `shift+pgup/pgdn`, `kitty_mod+home/end` |
| clear scrollback | `kitty_mod+delete` |
| font size | `kitty_mod+=` / `kitty_mod+-` / `kitty_mod+backspace`, or **Ctrl+mouse wheel** |
| new OS window | `kitty_mod+n` |
| fullscreen | `kitty_mod+f11` |
| export layout to `~/.config/dtem/sessions/` | `kitty_mod+alt+s` |
| search in scrollback | `kitty_mod+f` |
| settings page | `kitty_mod+f2`, `ctrl+,` |
| quit (asks; `d` = quit without saving) | `kitty_mod+alt+q` |
| transparency on/off | `toggle_transparency` (unbound by default) |
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
