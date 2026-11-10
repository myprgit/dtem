//! Font discovery via fontconfig, shaping (ligatures) and glyph lookup via swash.
use std::collections::HashMap;
use std::process::Command;
use std::sync::Arc;
use swash::shape::ShapeContext;
use swash::{CacheKey, FontRef};

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

pub struct Face {
    data: Arc<Vec<u8>>,
    offset: u32,
    key: CacheKey,
    /// Color font (emoji)
    pub color: bool,
}

/// JetBrains Mono (SIL OFL 1.1, see assets/fonts/JetBrainsMono-OFL.txt), built in so dtem
/// looks the same on any machine, even without fontconfig.
pub const BUILTIN_NAME: &str = "JetBrains Mono";
static BUILTIN: [&[u8]; 4] = [
    include_bytes!("../assets/fonts/JetBrainsMono-Regular.ttf"),
    include_bytes!("../assets/fonts/JetBrainsMono-Bold.ttf"),
    include_bytes!("../assets/fonts/JetBrainsMono-Italic.ttf"),
    include_bytes!("../assets/fonts/JetBrainsMono-BoldItalic.ttf"),
];

/// Family name fontconfig resolves a pattern to (None if fontconfig isn't available).
fn fc_family(pattern: &str) -> Option<String> {
    let out = Command::new("fc-match").args(["-f", "%{family}", pattern]).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).to_string())
}

/// Use the built-in font for this family? (explicitly, or JetBrains Mono not installed,
/// or no fontconfig at all)
fn use_builtin(family: &str) -> bool {
    let f = family.trim().to_lowercase();
    if f == "builtin" || f == "built-in" {
        return true;
    }
    if std::path::Path::new(family).is_file() {
        return false;
    }
    match fc_family(family) {
        None => true, // no fontconfig
        Some(found) => f == BUILTIN_NAME.to_lowercase() && !found.to_lowercase().contains(&f),
    }
}

impl Face {
    fn from_bytes(bytes: &[u8], color: bool) -> Option<Face> {
        let data = Arc::new(bytes.to_vec());
        let f = FontRef::from_index(&data, 0)?;
        let (offset, key) = (f.offset, f.key);
        Some(Face { data, offset, key, color })
    }

    fn load(path: &str, index: u32) -> Option<Face> {
        let data = Arc::new(std::fs::read(path).ok()?);
        let f = FontRef::from_index(&data, index as usize)?;
        let (offset, key) = (f.offset, f.key);
        let lower = path.to_lowercase();
        let color = lower.contains("emoji") || lower.contains("color");
        Some(Face { data, offset, key, color })
    }

    pub fn font(&self) -> FontRef<'_> {
        FontRef { data: &self.data, offset: self.offset, key: self.key }
    }

    pub fn glyph(&self, c: char) -> u16 {
        self.font().charmap().map(c)
    }
}

/// Process-wide set of faces. Face ids are indices into `faces`.
pub struct FontSet {
    faces: Vec<Face>,
    /// face ids for regular, bold, italic, bold-italic
    styles: [usize; 4],
    family: String,
    fallback_paths: HashMap<String, usize>,
    char_fallback: HashMap<char, Option<usize>>,
    shape_ctx: ShapeContext,
    shape_cache: HashMap<(usize, String), Arc<Vec<(u16, u16)>>>,
}

/// `font_family` may be a fontconfig name or a path to a font file.
fn load_family(family: &str, suffix: &str) -> Option<Face> {
    let p = std::path::Path::new(family);
    if p.is_file() {
        return if suffix.is_empty() || suffix == ":style=Regular" { Face::load(family, 0) } else { None };
    }
    let (f, i) = fc_match(&format!("{family}{suffix}"))?;
    Face::load(&f, i)
}

impl FontSet {
    pub fn new(family: &str) -> anyhow::Result<FontSet> {
        if use_builtin(family) {
            let faces: Vec<Face> = BUILTIN.iter().filter_map(|b| Face::from_bytes(b, false)).collect();
            anyhow::ensure!(faces.len() == 4, "built-in font is corrupt");
            return Ok(FontSet {
                faces,
                styles: [0, 1, 2, 3],
                family: "monospace".to_string(),
                fallback_paths: HashMap::new(),
                char_fallback: HashMap::new(),
                shape_ctx: ShapeContext::new(),
                shape_cache: HashMap::new(),
            });
        }
        let regular = load_family(family, ":style=Regular")
            .or_else(|| load_family(family, ""))
            .or_else(|| fc_match("monospace").and_then(|(f, i)| Face::load(&f, i)))
            .or_else(|| Face::from_bytes(BUILTIN[0], false))
            .ok_or_else(|| anyhow::anyhow!("no usable font found"))?;
        let mut faces = vec![regular];
        let mut styles = [0usize; 4];
        for (slot, suffix) in [(1usize, ":weight=bold"), (2, ":slant=italic"), (3, ":weight=bold:slant=italic")] {
            styles[slot] = match load_family(family, suffix) {
                Some(f) => {
                    faces.push(f);
                    faces.len() - 1
                }
                None => {
                    if slot == 3 {
                        styles[1]
                    } else {
                        0
                    }
                }
            };
        }
        Ok(FontSet {
            faces,
            styles,
            family: family.to_string(),
            fallback_paths: HashMap::new(),
            char_fallback: HashMap::new(),
            shape_ctx: ShapeContext::new(),
            shape_cache: HashMap::new(),
        })
    }

    pub fn face(&self, id: usize) -> &Face {
        &self.faces[id]
    }

    pub fn style_face(&self, style: u8) -> usize {
        self.styles[(style & 3) as usize]
    }

    /// Face id + glyph id able to render `c` in the given style.
    pub fn lookup(&mut self, c: char, style: u8) -> Option<(usize, u16)> {
        let sf = self.style_face(style);
        let g = self.faces[sf].glyph(c);
        if g != 0 {
            return Some((sf, g));
        }
        let g = self.faces[0].glyph(c);
        if g != 0 {
            return Some((0, g));
        }
        if let Some(r) = self.char_fallback.get(&c) {
            return r.map(|i| (i, self.faces[i].glyph(c)));
        }
        let mut found = (0..self.faces.len()).find(|&i| self.faces[i].glyph(c) != 0);
        if found.is_none() {
            let fam = if std::path::Path::new(&self.family).is_file() { "monospace" } else { &self.family };
            if let Some((path, idx)) = fc_match(&format!("{}:charset={:x}", fam, c as u32)) {
                let key = format!("{path}#{idx}");
                let id = match self.fallback_paths.get(&key) {
                    Some(i) => Some(*i),
                    None => Face::load(&path, idx).map(|f| {
                        self.faces.push(f);
                        let i = self.faces.len() - 1;
                        self.fallback_paths.insert(key, i);
                        i
                    }),
                };
                found = id.filter(|&i| self.faces[i].glyph(c) != 0);
            }
        }
        self.char_fallback.insert(c, found);
        found.map(|i| (i, self.faces[i].glyph(c)))
    }

    /// Shape a run of single-width cells (one char per cell). Returns (glyph id, cell index)
    /// pairs. Ligatures/contextual alternates come from the font's `liga`/`calt` features.
    pub fn shape(&mut self, face: usize, text: &str) -> Arc<Vec<(u16, u16)>> {
        let key = (face, text.to_string());
        if let Some(v) = self.shape_cache.get(&key) {
            return v.clone();
        }
        if self.shape_cache.len() > 20_000 {
            self.shape_cache.clear();
        }
        // byte offset -> cell index
        let mut cell_of = vec![0u16; text.len() + 1];
        for (cell, (b, ch)) in text.char_indices().enumerate() {
            for k in 0..ch.len_utf8() {
                cell_of[b + k] = cell as u16;
            }
        }
        let f = &self.faces[face];
        let font = FontRef { data: &f.data, offset: f.offset, key: f.key };
        let mut out = Vec::with_capacity(text.len());
        let mut shaper = self.shape_ctx.builder(font).features(&[("liga", 1), ("calt", 1)]).build();
        shaper.add_str(text);
        shaper.shape_with(|cluster| {
            let cell = cell_of[(cluster.source.start as usize).min(text.len())];
            for (i, g) in cluster.glyphs.iter().enumerate() {
                out.push((g.id, cell + i as u16));
            }
        });
        let v = Arc::new(out);
        self.shape_cache.insert(key, v.clone());
        v
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
    let font = fonts.face(0).font();
    let m = font.metrics(&[]).scale(px);
    let (asc, desc, gap) = (m.ascent, m.descent.abs(), m.leading);
    let natural = (asc + desc + gap).ceil();
    let height = (natural * line_height).round().max(1.0);
    let extra = ((height - natural) / 2.0).floor();
    let gm = font.glyph_metrics(&[]).scale(px);
    let width = gm.advance_width(font.charmap().map('M')).round().max(1.0);
    let ascent = (asc + gap / 2.0 + extra).round();
    let thickness = m.stroke_size.round().max((px / 14.0).round()).max(1.0);
    let underline_pos = (ascent - m.underline_offset).round().clamp(ascent + 1.0, height - thickness);
    CellMetrics { px, width, height, ascent, underline_pos, thickness }
}
