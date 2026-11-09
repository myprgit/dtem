//! PTY output filter: sits between the PTY and alacritty's parser and handles the escape
//! sequences alacritty ignores:
//!   OSC 7 (cwd), OSC 9 / OSC 777 (notifications), OSC 133 (shell integration marks),
//!   APC G (kitty graphics protocol).
//! Everything else passes through unchanged.
use crate::images::ImageStore;
use crate::UserEvent;
use alacritty_terminal::event::{OnResize, WindowSize};
use alacritty_terminal::tty::{ChildEvent, EventedPty, EventedReadWrite, Pty};
use parking_lot::Mutex;
use polling::{Event, PollMode, Poller};
use std::fs::File;
use std::io::{self, Read};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use winit::event_loop::EventLoopProxy;

#[derive(Debug, Clone)]
pub enum StreamEvent {
    /// OSC 7: file://host/path
    Cwd { host: String, path: String },
    /// OSC 9 / OSC 777;notify
    Notify { title: String, body: String },
    /// OSC 133;C — a command started producing output
    CommandStart,
    /// OSC 133;D[;exit]
    CommandEnd(Option<i32>),
    /// Bytes to send back to the PTY (kitty graphics replies)
    Reply(Vec<u8>),
}

/// Live geometry the filter needs (updated by the pane on resize).
#[derive(Default)]
pub struct Geometry {
    pub cell_w: AtomicU32,
    pub cell_h: AtomicU32,
    pub cols: AtomicU32,
}

enum State {
    Ground,
    Esc,
    Osc,
    OscEsc,
    Apc,
    ApcEsc,
}

pub struct FilterReader {
    file: File,
    state: State,
    seq: Vec<u8>,
    out: Vec<u8>,
    out_pos: usize,
    raw: Vec<u8>,
    pane: u64,
    proxy: EventLoopProxy<UserEvent>,
    images: Arc<Mutex<ImageStore>>,
    geo: Arc<Geometry>,
    mark: u64,
}

const MAX_SEQ: usize = 64 << 20;

impl FilterReader {
    fn emit(&self, ev: StreamEvent) {
        let _ = self.proxy.send_event(UserEvent::Stream(self.pane, ev));
    }

    fn process(&mut self, data: &[u8]) {
        for &b in data {
            match self.state {
                State::Ground => {
                    if b == 0x1b {
                        self.state = State::Esc;
                    } else {
                        self.out.push(b);
                    }
                }
                State::Esc => match b {
                    b']' => {
                        self.seq.clear();
                        self.state = State::Osc;
                    }
                    b'_' => {
                        self.seq.clear();
                        self.state = State::Apc;
                    }
                    0x1b => self.out.push(0x1b), // ESC ESC: emit the first, stay in Esc
                    _ => {
                        self.out.extend_from_slice(&[0x1b, b]);
                        self.state = State::Ground;
                    }
                },
                State::Osc => match b {
                    0x07 => self.finish_osc(b"\x07"),
                    0x1b => self.state = State::OscEsc,
                    _ => self.push_seq(b, b"\x1b]"),
                },
                State::OscEsc => {
                    if b == b'\\' {
                        self.finish_osc(b"\x1b\\");
                    } else {
                        // malformed: give the bytes to the parser as they were
                        self.out.extend_from_slice(b"\x1b]");
                        self.out.extend_from_slice(&self.seq);
                        self.seq.clear();
                        self.state = State::Esc;
                        self.process(&[b]);
                    }
                }
                State::Apc => match b {
                    0x1b => self.state = State::ApcEsc,
                    0x07 => self.finish_apc(),
                    _ => self.push_seq(b, b"\x1b_"),
                },
                State::ApcEsc => {
                    if b == b'\\' {
                        self.finish_apc();
                    } else {
                        self.seq.push(0x1b);
                        self.seq.push(b);
                        self.state = State::Apc;
                    }
                }
            }
        }
    }

    fn push_seq(&mut self, b: u8, intro: &[u8]) {
        self.seq.push(b);
        if self.seq.len() > MAX_SEQ {
            self.out.extend_from_slice(intro);
            self.out.append(&mut self.seq);
            self.state = State::Ground;
        }
    }

    fn finish_osc(&mut self, term: &[u8]) {
        self.state = State::Ground;
        let seq = std::mem::take(&mut self.seq);
        let text = String::from_utf8_lossy(&seq).to_string();
        let (num, rest) = text.split_once(';').unwrap_or((text.as_str(), ""));
        match num {
            "7" => {
                // file://host/path (percent-encoded)
                if let Some(u) = rest.strip_prefix("file://") {
                    let (host, path) = match u.find('/') {
                        Some(i) => (&u[..i], &u[i..]),
                        None => (u, "/"),
                    };
                    self.emit(StreamEvent::Cwd { host: host.to_string(), path: percent_decode(path) });
                }
            }
            "9" => {
                // ConEmu uses "9;4;..." for progress — ignore those
                if !rest.starts_with("4;") && !rest.is_empty() {
                    self.emit(StreamEvent::Notify { title: String::new(), body: rest.to_string() });
                }
            }
            "777" => {
                let mut it = rest.splitn(3, ';');
                if it.next() == Some("notify") {
                    let title = it.next().unwrap_or("").to_string();
                    let body = it.next().unwrap_or("").to_string();
                    self.emit(StreamEvent::Notify { title, body });
                }
            }
            "133" => {
                // Prompt/command regions become invisible OSC 8 links on the cells, so they
                // move with the text through scrolling and scrollback.
                let kind = rest.chars().next().unwrap_or(' ');
                match kind {
                    'A' => {
                        self.mark += 1;
                        let m = self.mark;
                        self.out.extend_from_slice(format!("\x1b]8;id=dtemp{m};dtem:prompt:{m}\x1b\\").as_bytes());
                    }
                    'B' => self.out.extend_from_slice(b"\x1b]8;;\x1b\\"),
                    'C' => {
                        let m = self.mark;
                        self.out.extend_from_slice(format!("\x1b]8;id=dtemo{m};dtem:output:{m}\x1b\\").as_bytes());
                        self.emit(StreamEvent::CommandStart);
                    }
                    'D' => {
                        self.out.extend_from_slice(b"\x1b]8;;\x1b\\");
                        let code = rest.split(';').nth(1).and_then(|c| c.trim().parse().ok());
                        self.emit(StreamEvent::CommandEnd(code));
                    }
                    _ => {}
                }
            }
            _ => {
                self.out.extend_from_slice(b"\x1b]");
                self.out.extend_from_slice(&seq);
                self.out.extend_from_slice(term);
            }
        }
    }

    fn finish_apc(&mut self) {
        self.state = State::Ground;
        let seq = std::mem::take(&mut self.seq);
        if seq.first() == Some(&b'G') {
            let cw = self.geo.cell_w.load(Ordering::Relaxed).max(1);
            let ch = self.geo.cell_h.load(Ordering::Relaxed).max(1);
            let cols = self.geo.cols.load(Ordering::Relaxed).max(1);
            let (out, reply) = self.images.lock().handle(&seq[1..], cw, ch, cols);
            self.out.extend_from_slice(&out);
            if let Some(r) = reply {
                self.emit(StreamEvent::Reply(r));
            }
        }
        // other APC strings are ignored (alacritty ignores them too)
    }
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

impl Read for FilterReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        loop {
            if self.out_pos < self.out.len() {
                let n = (self.out.len() - self.out_pos).min(buf.len());
                buf[..n].copy_from_slice(&self.out[self.out_pos..self.out_pos + n]);
                self.out_pos += n;
                if self.out_pos == self.out.len() {
                    self.out.clear();
                    self.out_pos = 0;
                }
                return Ok(n);
            }
            if self.raw.len() < buf.len().max(4096) {
                self.raw.resize(buf.len().max(4096), 0);
            }
            let n = self.file.read(&mut self.raw)?; // WouldBlock propagates
            if n == 0 {
                return Ok(0);
            }
            let raw = std::mem::take(&mut self.raw);
            self.process(&raw[..n]);
            self.raw = raw;
            // nothing to hand out yet (everything was inside an escape sequence): read more
        }
    }
}

/// The real PTY with our filtering reader.
pub struct FilteredPty {
    inner: Pty,
    reader: FilterReader,
}

impl FilteredPty {
    pub fn new(
        inner: Pty,
        pane: u64,
        proxy: EventLoopProxy<UserEvent>,
        images: Arc<Mutex<ImageStore>>,
        geo: Arc<Geometry>,
    ) -> io::Result<FilteredPty> {
        let file = inner.file().try_clone()?;
        Ok(FilteredPty {
            inner,
            reader: FilterReader {
                file,
                state: State::Ground,
                seq: Vec::new(),
                out: Vec::new(),
                out_pos: 0,
                raw: Vec::new(),
                pane,
                proxy,
                images,
                geo,
                mark: 0,
            },
        })
    }
}

impl EventedReadWrite for FilteredPty {
    type Reader = FilterReader;
    type Writer = File;

    unsafe fn register(&mut self, poll: &Arc<Poller>, interest: Event, mode: PollMode) -> io::Result<()> {
        unsafe { self.inner.register(poll, interest, mode) }
    }

    fn reregister(&mut self, poll: &Arc<Poller>, interest: Event, mode: PollMode) -> io::Result<()> {
        self.inner.reregister(poll, interest, mode)
    }

    fn deregister(&mut self, poll: &Arc<Poller>) -> io::Result<()> {
        self.inner.deregister(poll)
    }

    fn reader(&mut self) -> &mut FilterReader {
        &mut self.reader
    }

    fn writer(&mut self) -> &mut File {
        self.inner.writer()
    }
}

impl EventedPty for FilteredPty {
    fn next_child_event(&mut self) -> Option<ChildEvent> {
        self.inner.next_child_event()
    }
}

impl OnResize for FilteredPty {
    fn on_resize(&mut self, ws: WindowSize) {
        self.inner.on_resize(ws)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn decode() {
        assert_eq!(super::percent_decode("/home/a%20b/x"), "/home/a b/x");
        assert_eq!(super::percent_decode("/tmp/%"), "/tmp/%");
    }
}
