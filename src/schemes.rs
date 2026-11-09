//! Built-in color schemes (tilda's palettes + a few popular themes).
use crate::config::{parse_color, Rgb};

#[allow(dead_code)]
pub struct Scheme {
    pub name: &'static str,
    pub fg: Rgb,
    pub bg: Rgb,
    pub cursor: Option<Rgb>,
    pub palette: [Rgb; 16],
}

fn p(hex: [&str; 16]) -> [Rgb; 16] {
    let mut out = [[0u8; 3]; 16];
    for (o, h) in out.iter_mut().zip(hex) {
        *o = parse_color(h);
    }
    out
}

const TANGO: [&str; 16] = [
    "#2e3436", "#cc0000", "#4e9a06", "#c4a000", "#3465a4", "#75507b", "#06989a", "#d3d7cf", "#555753", "#ef2929",
    "#8ae234", "#fce94f", "#729fcf", "#ad7fa8", "#34e2e2", "#eeeeec",
];
const LINUX: [&str; 16] = [
    "#000000", "#aa0000", "#00aa00", "#aa5500", "#0000aa", "#aa00aa", "#00aaaa", "#aaaaaa", "#555555", "#ff5555",
    "#55ff55", "#ffff55", "#5555ff", "#ff55ff", "#55ffff", "#ffffff",
];
const XTERM: [&str; 16] = [
    "#000000", "#cd0000", "#00cd00", "#cdcd00", "#1e90ff", "#cd00cd", "#00cdcd", "#e5e5e5", "#4c4c4c", "#ff0000",
    "#00ff00", "#ffff00", "#4682b4", "#ff00ff", "#00ffff", "#ffffff",
];
const RXVT: [&str; 16] = [
    "#000000", "#cd0000", "#00cd00", "#cdcd00", "#0000cd", "#cd00cd", "#00cdcd", "#faebd7", "#404040", "#ff0000",
    "#00ff00", "#ffff00", "#0000ff", "#ff00ff", "#00ffff", "#ffffff",
];
const ZENBURN: [&str; 16] = [
    "#222222", "#803232", "#5b762f", "#aa9943", "#324c80", "#706c9a", "#92b19e", "#ffffff", "#222222", "#982b2b",
    "#89b83f", "#efef60", "#2b4f98", "#826ab1", "#a1cdcd", "#dedede",
];
const SOLARIZED_D: [&str; 16] = [
    "#073642", "#dc322f", "#859900", "#b58900", "#268bd2", "#d33682", "#2aa198", "#eee8d5", "#002b36", "#cb4b16",
    "#586e75", "#839496", "#657b83", "#6c71c4", "#93a1a1", "#fdf6e3",
];
const SOLARIZED_L: [&str; 16] = [
    "#eee8d5", "#dc322f", "#859900", "#b58900", "#268bd2", "#d33682", "#2aa198", "#073642", "#fdf6e3", "#cb4b16",
    "#93a1a1", "#839496", "#657b83", "#6c71c4", "#586e75", "#002b36",
];
const SNAZZY: [&str; 16] = [
    "#282a36", "#ff5c57", "#5af78e", "#f3f99d", "#57c7ff", "#ff6ac1", "#9aedfe", "#f1f1f0", "#686868", "#ff5c57",
    "#5af78e", "#f3f99d", "#57c7ff", "#ff6ac1", "#9aedfe", "#f1f1f0",
];
const TOKYO: [&str; 16] = [
    "#15161e", "#f7768e", "#9ece6a", "#e0af68", "#7aa2f7", "#bb9af7", "#7dcfff", "#a9b1d6", "#414868", "#f7768e",
    "#9ece6a", "#e0af68", "#7aa2f7", "#bb9af7", "#7dcfff", "#c0caf5",
];
const DRACULA: [&str; 16] = [
    "#21222c", "#ff5555", "#50fa7b", "#f1fa8c", "#bd93f9", "#ff79c6", "#8be9fd", "#f8f8f2", "#6272a4", "#ff6e6e",
    "#69ff94", "#ffffa5", "#d6acff", "#ff92df", "#a4ffff", "#ffffff",
];
const GRUVBOX: [&str; 16] = [
    "#282828", "#cc241d", "#98971a", "#d79921", "#458588", "#b16286", "#689d6a", "#a89984", "#928374", "#fb4934",
    "#b8bb26", "#fabd2f", "#83a598", "#d3869b", "#8ec07c", "#ebdbb2",
];
const NORD: [&str; 16] = [
    "#3b4252", "#bf616a", "#a3be8c", "#ebcb8b", "#81a1c1", "#b48ead", "#88c0d0", "#e5e9f0", "#4c566a", "#bf616a",
    "#a3be8c", "#ebcb8b", "#81a1c1", "#b48ead", "#8fbcbb", "#eceff4",
];
const CATPPUCCIN: [&str; 16] = [
    "#45475a", "#f38ba8", "#a6e3a1", "#f9e2af", "#89b4fa", "#f5c2e7", "#94e2d5", "#bac2de", "#585b70", "#f38ba8",
    "#a6e3a1", "#f9e2af", "#89b4fa", "#f5c2e7", "#94e2d5", "#a6adc8",
];
const ONE_DARK: [&str; 16] = [
    "#282c34", "#e06c75", "#98c379", "#e5c07b", "#61afef", "#c678dd", "#56b6c2", "#abb2bf", "#5c6370", "#e06c75",
    "#98c379", "#e5c07b", "#61afef", "#c678dd", "#56b6c2", "#ffffff",
];

pub const NAMES: &[&str] = &[
    "tokyo-night",
    "tango",
    "white-on-black",
    "green-on-black",
    "black-on-white",
    "linux",
    "xterm",
    "rxvt",
    "zenburn",
    "solarized-dark",
    "solarized-light",
    "snazzy",
    "dracula",
    "gruvbox-dark",
    "nord",
    "catppuccin-mocha",
    "one-dark",
];

pub fn get(name: &str) -> Option<Scheme> {
    let c = parse_color;
    let s = |name, fg, bg, cursor: Option<&str>, pal| Scheme { name, fg: c(fg), bg: c(bg), cursor: cursor.map(c), palette: p(pal) };
    Some(match name.trim().to_lowercase().as_str() {
        "" | "tokyo-night" => s("tokyo-night", "#dddddd", "#1a1b26", Some("#c0caf5"), TOKYO),
        "tango" | "white-on-black" => s("white-on-black", "#ffffff", "#000000", None, TANGO),
        "green-on-black" => s("green-on-black", "#00ff00", "#000000", None, TANGO),
        "black-on-white" => s("black-on-white", "#000000", "#ffffff", None, TANGO),
        "linux" => s("linux", "#aaaaaa", "#000000", None, LINUX),
        "xterm" => s("xterm", "#e5e5e5", "#000000", None, XTERM),
        "rxvt" => s("rxvt", "#faebd7", "#000000", None, RXVT),
        "zenburn" => s("zenburn", "#dca3a3", "#3f3f3f", None, ZENBURN),
        "solarized-dark" | "solarized" => s("solarized-dark", "#839496", "#002b36", None, SOLARIZED_D),
        "solarized-light" => s("solarized-light", "#657b83", "#fdf6e3", None, SOLARIZED_L),
        "snazzy" => s("snazzy", "#eff0eb", "#282a36", None, SNAZZY),
        "dracula" => s("dracula", "#f8f8f2", "#282a36", None, DRACULA),
        "gruvbox-dark" | "gruvbox" => s("gruvbox-dark", "#ebdbb2", "#282828", None, GRUVBOX),
        "nord" => s("nord", "#d8dee9", "#2e3440", None, NORD),
        "catppuccin-mocha" | "catppuccin" => s("catppuccin-mocha", "#cdd6f4", "#1e1e2e", Some("#f5e0dc"), CATPPUCCIN),
        "one-dark" => s("one-dark", "#abb2bf", "#282c34", None, ONE_DARK),
        _ => return None,
    })
}

pub fn default_scheme() -> Scheme {
    get("tokyo-night").unwrap()
}
