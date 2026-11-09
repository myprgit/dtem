//! kitty graphics protocol (subset): transmit / display / delete / query.
//!
//! Displayed images are written into the grid as placeholder cells (U+10EEEE) whose
//! foreground color carries the placement key and background color the (row, col) inside
//! the image. The renderer draws the image slice for every such cell, so images scroll,
//! live in scrollback and are cleared exactly like text.
use base64::Engine;
use std::collections::HashMap;
use std::fmt::Write as _;
use std::io::Read;
use std::sync::Arc;

pub const PLACEHOLDER: char = '\u{10EEEE}';
const MAX_IMAGE_BYTES: usize = 200 << 20;

pub struct Image {
    pub w: u32,
    pub h: u32,
    pub rgba: Arc<Vec<u8>>,
    pub version: u64,
}

#[derive(Clone, Copy)]
pub struct Placement {
    pub image: u32,
    pub cols: u32,
    pub rows: u32,
}

#[derive(Default)]
pub struct ImageStore {
    pub images: HashMap<u32, Image>,
    /// placement key (fits in 24 bits) → placement
    pub placements: HashMap<u32, Placement>,
    numbers: HashMap<u32, u32>,
    pending: Option<(HashMap<char, String>, Vec<u8>)>,
    next_id: u32,
    next_key: u32,
    version: u64,
}

fn parse_keys(s: &str) -> HashMap<char, String> {
    s.split(',')
        .filter_map(|kv| {
            let (k, v) = kv.split_once('=')?;
            Some((k.chars().next()?, v.to_string()))
        })
        .collect()
}

impl ImageStore {
    fn num(keys: &HashMap<char, String>, k: char) -> u32 {
        keys.get(&k).and_then(|v| v.parse().ok()).unwrap_or(0)
    }

    /// Handle one `G...` command. Returns (bytes to inject into the output, reply to the app).
    pub fn handle(&mut self, cmd: &[u8], cw: u32, ch: u32, term_cols: u32) -> (Vec<u8>, Option<Vec<u8>>) {
        let text = String::from_utf8_lossy(cmd);
        let (ctrl, payload) = text.split_once(';').unwrap_or((&text, ""));
        let mut keys = parse_keys(ctrl);

        // chunked transmission
        let more = keys.get(&'m').map(|v| v == "1").unwrap_or(false);
        if let Some((first, mut data)) = self.pending.take() {
            data.extend_from_slice(payload.as_bytes());
            if more {
                self.pending = Some((first, data));
                return (Vec::new(), None);
            }
            let q = keys.get(&'q').cloned();
            keys = first;
            if let Some(q) = q {
                keys.insert('q', q);
            }
            return self.execute(keys, &data, cw, ch, term_cols);
        }
        if more {
            self.pending = Some((keys, payload.as_bytes().to_vec()));
            return (Vec::new(), None);
        }
        self.execute(keys, payload.as_bytes(), cw, ch, term_cols)
    }

    fn reply(keys: &HashMap<char, String>, msg: &str) -> Option<Vec<u8>> {
        let q = Self::num(keys, 'q');
        let ok = msg == "OK";
        if q >= 2 || (q == 1 && ok) {
            return None;
        }
        let i = Self::num(keys, 'i');
        let n = Self::num(keys, 'I');
        if i == 0 && n == 0 {
            return None;
        }
        let mut s = String::from("\x1b_G");
        if i != 0 {
            let _ = write!(s, "i={i}");
        }
        if n != 0 {
            let _ = write!(s, "{}I={n}", if i != 0 { "," } else { "" });
        }
        let _ = write!(s, ";{msg}\x1b\\");
        Some(s.into_bytes())
    }

    fn execute(&mut self, keys: HashMap<char, String>, payload: &[u8], cw: u32, ch: u32, term_cols: u32) -> (Vec<u8>, Option<Vec<u8>>) {
        let action = keys.get(&'a').and_then(|v| v.chars().next()).unwrap_or('t');
        match action {
            'q' => match decode(&keys, payload) {
                Ok(_) => (Vec::new(), Self::reply(&keys, "OK")),
                Err(e) => (Vec::new(), Self::reply(&keys, &e)),
            },
            't' | 'T' => {
                let img = match decode(&keys, payload) {
                    Ok(i) => i,
                    Err(e) => return (Vec::new(), Self::reply(&keys, &e)),
                };
                let id = self.store_id(&keys);
                self.version += 1;
                self.images.insert(id, Image { w: img.0, h: img.1, rgba: Arc::new(img.2), version: self.version });
                // keep memory bounded
                if self.images.len() > 256 {
                    let oldest = self.images.iter().min_by_key(|(_, i)| i.version).map(|(k, _)| *k);
                    if let Some(k) = oldest {
                        self.images.remove(&k);
                    }
                }
                let mut keys = keys;
                keys.insert('i', id.to_string());
                let reply = Self::reply(&keys, "OK");
                if action == 'T' {
                    (self.place(id, &keys, cw, ch, term_cols), reply)
                } else {
                    (Vec::new(), reply)
                }
            }
            'p' => {
                let id = self.lookup_id(&keys);
                match id.filter(|i| self.images.contains_key(i)) {
                    Some(id) => (self.place(id, &keys, cw, ch, term_cols), Self::reply(&keys, "OK")),
                    None => (Vec::new(), Self::reply(&keys, "ENOENT:image not found")),
                }
            }
            'd' => {
                let what = keys.get(&'d').and_then(|v| v.chars().next()).unwrap_or('a');
                match what {
                    'i' | 'I' => {
                        if let Some(id) = self.lookup_id(&keys) {
                            if what == 'I' {
                                self.images.remove(&id);
                            }
                            self.placements.retain(|_, p| p.image != id);
                        }
                    }
                    'A' => {
                        self.images.clear();
                        self.placements.clear();
                    }
                    _ => self.placements.clear(),
                }
                (Vec::new(), None)
            }
            _ => (Vec::new(), None),
        }
    }

    fn store_id(&mut self, keys: &HashMap<char, String>) -> u32 {
        let i = Self::num(keys, 'i');
        if i != 0 {
            return i;
        }
        self.next_id += 1;
        let id = 0x8000_0000 | self.next_id;
        let n = Self::num(keys, 'I');
        if n != 0 {
            self.numbers.insert(n, id);
        }
        id
    }

    fn lookup_id(&self, keys: &HashMap<char, String>) -> Option<u32> {
        let i = Self::num(keys, 'i');
        if i != 0 {
            return Some(i);
        }
        self.numbers.get(&Self::num(keys, 'I')).copied()
    }

    /// Emit placeholder cells for a placement.
    fn place(&mut self, id: u32, keys: &HashMap<char, String>, cw: u32, ch: u32, term_cols: u32) -> Vec<u8> {
        let Some(img) = self.images.get(&id) else { return Vec::new() };
        let (mut cols, mut rows) = (Self::num(keys, 'c'), Self::num(keys, 'r'));
        let (iw, ih) = (img.w.max(1), img.h.max(1));
        match (cols, rows) {
            (0, 0) => {
                cols = iw.div_ceil(cw);
                rows = ih.div_ceil(ch);
            }
            (c, 0) => rows = ((c * cw) as f64 * ih as f64 / iw as f64 / ch as f64).ceil() as u32,
            (0, r) => cols = ((r * ch) as f64 * iw as f64 / ih as f64 / cw as f64).ceil() as u32,
            _ => {}
        }
        cols = cols.clamp(1, term_cols.max(1)).min(4095);
        rows = rows.clamp(1, 4095);
        self.next_key = (self.next_key % 0xFF_FFFE) + 1;
        let key = self.next_key;
        self.placements.insert(key, Placement { image: id, cols, rows });
        if self.placements.len() > 4096 {
            // keep the most recent 4096 placements (keys wrap around)
            let dist = |k: u32| (key + 0xFF_FFFE - k) % 0xFF_FFFE;
            self.placements.retain(|k, _| dist(*k) < 4096);
        }
        let no_move = Self::num(keys, 'C') == 1;
        let mut s = String::new();
        if no_move {
            s.push_str("\x1b7");
        }
        let (kr, kg, kb) = (key >> 16 & 255, key >> 8 & 255, key & 255);
        for r in 0..rows {
            let _ = write!(s, "\x1b[38;2;{kr};{kg};{kb}m");
            for c in 0..cols {
                let v = (r << 12) | c;
                let _ = write!(s, "\x1b[48;2;{};{};{}m{PLACEHOLDER}", v >> 16 & 255, v >> 8 & 255, v & 255);
            }
            s.push_str("\x1b[39;49m");
            if r + 1 < rows {
                let _ = write!(s, "\n\x1b[{cols}D");
            }
        }
        if no_move {
            s.push_str("\x1b8");
        }
        s.into_bytes()
    }
}

/// Decode the payload into (w, h, rgba).
fn decode(keys: &HashMap<char, String>, payload: &[u8]) -> Result<(u32, u32, Vec<u8>), String> {
    let clean: Vec<u8> = payload.iter().copied().filter(|b| !b.is_ascii_whitespace()).collect();
    let raw = base64::engine::general_purpose::STANDARD
        .decode(&clean)
        .or_else(|_| base64::engine::general_purpose::STANDARD_NO_PAD.decode(&clean))
        .map_err(|_| "EINVAL:bad base64".to_string())?;
    let medium = keys.get(&'t').and_then(|v| v.chars().next()).unwrap_or('d');
    let mut data = match medium {
        'd' => raw,
        'f' | 't' | 's' => {
            let name = String::from_utf8_lossy(&raw).to_string();
            let path = if medium == 's' && !name.starts_with('/') { format!("/dev/shm/{name}") } else { name.clone() };
            let d = std::fs::read(&path).map_err(|e| format!("EBADF:{e}"))?;
            let temp_ok = path.contains("tty-graphics-protocol") && (path.starts_with("/tmp/") || path.starts_with("/dev/shm/"));
            if medium == 's' || (medium == 't' && temp_ok) {
                let _ = std::fs::remove_file(&path);
            }
            d
        }
        _ => return Err("EINVAL:unsupported medium".into()),
    };
    if keys.get(&'o').map(|v| v == "z").unwrap_or(false) {
        let mut out = Vec::new();
        flate2::read::ZlibDecoder::new(&data[..])
            .take(MAX_IMAGE_BYTES as u64)
            .read_to_end(&mut out)
            .map_err(|_| "EINVAL:bad zlib data".to_string())?;
        data = out;
    }
    let num = |k: char| keys.get(&k).and_then(|v| v.parse::<u32>().ok()).unwrap_or(0);
    let format = num('f');
    let (w, h, mut rgba) = match format {
        100 => {
            let img = image::load_from_memory(&data).map_err(|e| format!("EINVAL:{e}"))?.to_rgba8();
            let (w, h) = img.dimensions();
            (w, h, img.into_raw())
        }
        24 | 0 | 32 => {
            let (w, h) = (num('s'), num('v'));
            let bpp = if format == 24 { 3 } else { 4 };
            if w == 0 || h == 0 || data.len() < (w * h) as usize * bpp {
                return Err("EINVAL:size mismatch".into());
            }
            let rgba = if bpp == 3 {
                data.chunks(3).take((w * h) as usize).flat_map(|p| [p[0], p[1], p[2], 255]).collect()
            } else {
                data[..(w * h * 4) as usize].to_vec()
            };
            (w, h, rgba)
        }
        _ => return Err("EINVAL:unsupported format".into()),
    };
    if (w as usize) * (h as usize) * 4 > MAX_IMAGE_BYTES {
        return Err("EFBIG:image too large".into());
    }
    // source rectangle
    let (x, y, sw, sh) = (num('x'), num('y'), num('w'), num('h'));
    if x > 0 || y > 0 || sw > 0 || sh > 0 {
        let sw = if sw == 0 { w.saturating_sub(x) } else { sw.min(w.saturating_sub(x)) };
        let sh = if sh == 0 { h.saturating_sub(y) } else { sh.min(h.saturating_sub(y)) };
        if sw > 0 && sh > 0 {
            let mut out = Vec::with_capacity((sw * sh * 4) as usize);
            for row in y..y + sh {
                let start = ((row * w + x) * 4) as usize;
                out.extend_from_slice(&rgba[start..start + (sw * 4) as usize]);
            }
            rgba = out;
            return Ok((sw, sh, rgba));
        }
    }
    Ok((w, h, rgba))
}

/// Decode placement key and (row, col) from a placeholder cell's colors.
pub fn decode_cell(fg: [u8; 3], bg: [u8; 3]) -> (u32, u32, u32) {
    let key = (fg[0] as u32) << 16 | (fg[1] as u32) << 8 | fg[2] as u32;
    let v = (bg[0] as u32) << 16 | (bg[1] as u32) << 8 | bg[2] as u32;
    (key, v >> 12, v & 0xFFF)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn transmit_and_place() {
        let mut st = ImageStore::default();
        // 2x1 RGBA image, displayed: 2 cols x 1 row with 10px cells
        let data = base64::engine::general_purpose::STANDARD.encode([255u8, 0, 0, 255, 0, 255, 0, 255]);
        let cmd = format!("a=T,f=32,s=2,v=1,i=7;{data}");
        let (out, reply) = st.handle(cmd.as_bytes(), 1, 1, 80);
        let out = String::from_utf8(out).unwrap();
        assert_eq!(out.matches(PLACEHOLDER).count(), 2);
        assert_eq!(reply.unwrap(), b"\x1b_Gi=7;OK\x1b\\".to_vec());
        assert_eq!(st.images[&7].w, 2);
        // chunked + query
        let (_, r) = st.handle(b"a=q,i=9,m=1,f=32,s=1,v=1;AAAA", 1, 1, 80);
        assert!(r.is_none());
        let (_, r) = st.handle(b"m=0;AA==", 1, 1, 80);
        assert_eq!(r.unwrap(), b"\x1b_Gi=9;OK\x1b\\".to_vec());
        assert_eq!(decode_cell([0, 0, 1], [0, 0x10, 0x03]), (1, 1, 3));
    }
}
