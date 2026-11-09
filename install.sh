#!/usr/bin/env bash
# Build dtem and install it for the current user.
#   ./install.sh            binary + desktop entry + dropdown autostart
#   ./install.sh --shortcut also register the dropdown hotkey in GNOME (works on X11 and Wayland)
set -euo pipefail
cd "$(dirname "$0")"

cargo build --release
# replace atomically (works while dtem is running; running instances keep the old binary)
mkdir -p "$HOME/.local/bin"
install -m755 target/release/dtem "$HOME/.local/bin/.dtem.new"
mv -f "$HOME/.local/bin/.dtem.new" "$HOME/.local/bin/dtem"

apps="$HOME/.local/share/applications"
mkdir -p "$apps" "$HOME/.config/autostart" "$HOME/.config/dtem"

cat > "$apps/dtem.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=dtem
Comment=GPU accelerated terminal
Exec=$HOME/.local/bin/dtem
Icon=utilities-terminal
Terminal=false
Categories=System;TerminalEmulator;
StartupWMClass=dtem
EOF

# Dropdown starts hidden at login and restores its tabs (reboot-safe).
cat > "$HOME/.config/autostart/dtem-dropdown.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=dtem dropdown
Exec=$HOME/.local/bin/dtem --dropdown --hidden
X-GNOME-Autostart-enabled=true
NoDisplay=true
EOF

if [ ! -f "$HOME/.config/dtem/dtem.toml" ]; then
  cp dtem.example.toml "$HOME/.config/dtem/dtem.toml"
fi

if [ "${1:-}" = "--shortcut" ]; then
  "$HOME/.local/bin/dtem" --install-gnome-shortcut
fi

echo "Installed: ~/.local/bin/dtem"
