//! Font discovery via fontconfig (fc-match) and rasterization via fontdue.
use fontdue::{Font, FontSettings, Metrics};
use std::collections::HashMap;
use std::process::Command;
use std::sync::Arc;

pub const REGULAR: u8 = 0;
pub const BOLD: u8 = 1;
pub const ITALIC: u8 = 2;

fn fc_match(pattern: &str) -> Option<(String, u32)> {
    let out = Command::new("fc-match").args(["-f", "%{file}\n%{index}", pattern]).output().ok()?;
    let s = String::from_utf8_lossy(&out.stdout).to_string();
    let mut it = s.lines();
    let file = it.next()?.trim().to_string();
    let idx = it.next().and_then(|i| i.trim().parse().ok()).unwrap_or(0);
    if file.is_empty() {
        None
    } else {
        Some((file, idx))
    }
}

fn load(path: &str, index: u32) -> Option<Arc<Font>> {
    let lower = path.to_lowercase();
    // fontdue can't render color bitmap fonts; skip them.
    if lower.contains("emoji") && lower.contains("color") {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    let settings = FontSettings { collection_index: index, scale: 40.0, ..Default::default() };
    Font::from_bytes(bytes, settings).ok().map(Arc::new)
}

/// Shared set of fonts (process-wide). Rasterization happens per window at its own px size.
pub struct FontSet {
    styles: [Arc<Font>; 4],
    family: String,
    fallbacks: Vec<Arc<Font>>,
    fallback_paths: HashMap<String, usize>,
    char_fallback: HashMap<char, Option<usize>>,
}

impl FontSet {
    pub fn new(family: &str) -> anyhow::Result<FontSet> {
        let get = |suffix: &str| -> Option<Arc<Font>> {
            let (f, i) = fc_match(&format!("{family}{suffix}"))?;
            load(&f, i)
        };
        let regular = get(":style=Regular")
            .or_else(|| get(""))
            .or_else(|| fc_match("monospace").and_then(|(f, i)| load(&f, i)))
            .ok_or_else(|| anyhow::anyhow!("no usable font found (is fontconfig installed?)"))?;
        let bold = get(":weight=bold").unwrap_or_else(|| regular.clone());
        let italic = get(":slant=italic").unwrap_or_else(|| regular.clone());
        let bold_italic = get(":weight=bold:slant=italic").unwrap_or_else(|| bold.clone());
        Ok(FontSet {
            styles: [regular, bold, italic, bold_italic],
            family: family.to_string(),
            fallbacks: Vec::new(),
            fallback_paths: HashMap::new(),
            char_fallback: HashMap::new(),
        })
    }

    pub fn regular(&self) -> &Font {
        &self.styles[0]
    }

    /// Returns the font that can render `c` for the style.
    pub fn font_for(&mut self, c: char, style: u8) -> Option<Arc<Font>> {
        let f = &self.styles[(style & 3) as usize];
        if f.lookup_glyph_index(c) != 0 {
            return Some(f.clone());
        }
        if self.styles[0].lookup_glyph_index(c) != 0 {
            return Some(self.styles[0].clone());
        }
        if let Some(r) = self.char_fallback.get(&c) {
            return r.map(|i| self.fallbacks[i].clone());
        }
        // Try already-loaded fallbacks first
        let mut found = self.fallbacks.iter().position(|f| f.lookup_glyph_index(c) != 0);
        if found.is_none() {
            let pat = format!("{}:charset={:x}", self.family, c as u32);
            if let Some((path, idx)) = fc_match(&pat) {
                let key = format!("{path}#{idx}");
                let i = match self.fallback_paths.get(&key) {
                    Some(i) => Some(*i),
                    None => load(&path, idx).map(|f| {
                        self.fallbacks.push(f);
                        let i = self.fallbacks.len() - 1;
                        self.fallback_paths.insert(key, i);
                        i
                    }),
                };
                if let Some(i) = i {
                    if self.fallbacks[i].lookup_glyph_index(c) != 0 {
                        found = Some(i);
                    }
                }
            }
        }
        self.char_fallback.insert(c, found);
        found.map(|i| self.fallbacks[i].clone())
    }
}

#[derive(Clone, Copy, Debug)]
pub struct CellMetrics {
    pub px: f32,
    pub width: f32,
    pub height: f32,
    /// distance from top of cell to baseline
    pub ascent: f32,
    pub underline_pos: f32,
    pub thickness: f32,
}

pub fn cell_metrics(fonts: &FontSet, px: f32, line_height: f32) -> CellMetrics {
    let f = fonts.regular();
    let lm = f.horizontal_line_metrics(px);
    let (asc, desc, gap) = lm.map(|m| (m.ascent, m.descent, m.line_gap)).unwrap_or((px * 0.8, -px * 0.2, 0.0));
    let natural = (asc - desc + gap).ceil();
    let height = (natural * line_height).round().max(1.0);
    let extra = ((height - natural) / 2.0).floor();
    let width = f.metrics('M', px).advance_width.round().max(1.0);
    let ascent = (asc + gap / 2.0 + extra).round();
    let thickness = (px / 14.0).round().max(1.0);
    CellMetrics { px, width, height, ascent, underline_pos: (ascent + thickness * 1.5).min(height - thickness), thickness }
}

pub fn rasterize(font: &Font, c: char, px: f32) -> (Metrics, Vec<u8>) {
    font.rasterize(c, px)
}
