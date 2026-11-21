//! E-ink terminal: /bin/sh in a pty, a VT100/xterm-subset emulator built on `vte`,
//! drawn in a monospace font. Output is batched (~250 ms) since each e-ink refresh is slow.

use crate::canvas::*;
use crate::sys::Sys;
use crate::ui::*;
use std::os::fd::RawFd;
use std::time::Instant;

#[derive(Clone, Copy)]
struct Cell {
    ch: char,
    rev: bool,
}

const BLANK: Cell = Cell { ch: ' ', rev: false };

pub struct Grid {
    cols: usize,
    rows: usize,
    cells: Vec<Vec<Cell>>,
    cx: usize,
    cy: usize,
    rev: bool,
    top: usize,
    bot: usize,
    saved: (usize, usize),
    alt: Option<Vec<Vec<Cell>>>,
    pub cursor_visible: bool,
    wrap_pending: bool,
    pub responses: Vec<u8>,
}

impl Grid {
    pub fn new(cols: usize, rows: usize) -> Grid {
        Grid { cols, rows, cells: vec![vec![BLANK; cols]; rows], cx: 0, cy: 0, rev: false, top: 0, bot: rows - 1,
               saved: (0, 0), alt: None, cursor_visible: true, wrap_pending: false, responses: vec![] }
    }

    pub fn resize(&mut self, cols: usize, rows: usize) {
        if cols == self.cols && rows == self.rows {
            return;
        }
        for row in self.cells.iter_mut() {
            row.resize(cols, BLANK);
        }
        while self.cells.len() > rows {
            self.cells.remove(0);
            self.cy = self.cy.saturating_sub(1);
        }
        self.cells.resize(rows, vec![BLANK; cols]);
        self.cols = cols;
        self.rows = rows;
        self.top = 0;
        self.bot = rows - 1;
        self.cx = self.cx.min(cols - 1);
        self.cy = self.cy.min(rows - 1);
    }

    pub fn row_text(&self, r: usize) -> (String, Vec<bool>) {
        (self.cells[r].iter().map(|c| c.ch).collect(), self.cells[r].iter().map(|c| c.rev).collect())
    }

    pub fn cursor(&self) -> (usize, usize) {
        (self.cx, self.cy)
    }

    fn blank(&self) -> Cell {
        Cell { ch: ' ', rev: false }
    }

    fn scroll_up(&mut self, n: usize) {
        for _ in 0..n {
            self.cells.remove(self.top);
            self.cells.insert(self.bot, vec![self.blank(); self.cols]);
        }
    }

    fn scroll_down(&mut self, n: usize) {
        for _ in 0..n {
            self.cells.remove(self.bot);
            self.cells.insert(self.top, vec![self.blank(); self.cols]);
        }
    }

    fn linefeed(&mut self) {
        if self.cy == self.bot {
            self.scroll_up(1);
        } else if self.cy + 1 < self.rows {
            self.cy += 1;
        }
    }

    fn erase(&mut self, r: usize, from: usize, to: usize) {
        let b = self.blank();
        for c in from..to.min(self.cols) {
            self.cells[r][c] = b;
        }
    }
}

fn arg(params: &vte::Params, i: usize, default: usize) -> usize {
    params.iter().nth(i).and_then(|p| p.first().copied()).map(|v| v as usize).filter(|&v| v != 0).unwrap_or(default)
}

impl vte::Perform for Grid {
    fn print(&mut self, c: char) {
        if self.wrap_pending {
            self.wrap_pending = false;
            self.cx = 0;
            self.linefeed();
        }
        self.cells[self.cy][self.cx] = Cell { ch: c, rev: self.rev };
        if self.cx + 1 >= self.cols {
            self.wrap_pending = true;
        } else {
            self.cx += 1;
        }
    }

    fn execute(&mut self, b: u8) {
        self.wrap_pending = false;
        match b {
            b'\n' | 0x0b | 0x0c => self.linefeed(),
            b'\r' => self.cx = 0,
            0x08 => self.cx = self.cx.saturating_sub(1),
            b'\t' => self.cx = ((self.cx / 8 + 1) * 8).min(self.cols - 1),
            _ => {}
        }
    }

    fn csi_dispatch(&mut self, params: &vte::Params, inter: &[u8], _ignore: bool, action: char) {
        self.wrap_pending = false;
        let private = inter.first() == Some(&b'?');
        let (cols, rows) = (self.cols, self.rows);
        match action {
            'A' => self.cy = self.cy.saturating_sub(arg(params, 0, 1)).max(if self.cy >= self.top { self.top } else { 0 }),
            'B' => self.cy = (self.cy + arg(params, 0, 1)).min(if self.cy <= self.bot { self.bot } else { rows - 1 }),
            'C' => self.cx = (self.cx + arg(params, 0, 1)).min(cols - 1),
            'D' => self.cx = self.cx.saturating_sub(arg(params, 0, 1)),
            'E' => {
                self.cy = (self.cy + arg(params, 0, 1)).min(rows - 1);
                self.cx = 0;
            }
            'F' => {
                self.cy = self.cy.saturating_sub(arg(params, 0, 1));
                self.cx = 0;
            }
            'G' | '`' => self.cx = (arg(params, 0, 1) - 1).min(cols - 1),
            'd' => self.cy = (arg(params, 0, 1) - 1).min(rows - 1),
            'H' | 'f' => {
                self.cy = (arg(params, 0, 1) - 1).min(rows - 1);
                self.cx = (arg(params, 1, 1) - 1).min(cols - 1);
            }
            'J' => {
                let mode = params.iter().next().and_then(|p| p.first().copied()).unwrap_or(0);
                match mode {
                    0 => {
                        self.erase(self.cy, self.cx, cols);
                        for r in self.cy + 1..rows {
                            self.erase(r, 0, cols);
                        }
                    }
                    1 => {
                        for r in 0..self.cy {
                            self.erase(r, 0, cols);
                        }
                        self.erase(self.cy, 0, self.cx + 1);
                    }
                    _ => {
                        for r in 0..rows {
                            self.erase(r, 0, cols);
                        }
                    }
                }
            }
            'K' => {
                let mode = params.iter().next().and_then(|p| p.first().copied()).unwrap_or(0);
                match mode {
                    0 => self.erase(self.cy, self.cx, cols),
                    1 => self.erase(self.cy, 0, self.cx + 1),
                    _ => self.erase(self.cy, 0, cols),
                }
            }
            'X' => self.erase(self.cy, self.cx, self.cx + arg(params, 0, 1)),
            'L' if self.cy >= self.top && self.cy <= self.bot => {
                for _ in 0..arg(params, 0, 1) {
                    self.cells.remove(self.bot);
                    self.cells.insert(self.cy, vec![BLANK; cols]);
                }
            }
            'M' if self.cy >= self.top && self.cy <= self.bot => {
                for _ in 0..arg(params, 0, 1) {
                    self.cells.remove(self.cy);
                    self.cells.insert(self.bot, vec![BLANK; cols]);
                }
            }
            'P' => {
                for _ in 0..arg(params, 0, 1).min(cols - self.cx) {
                    self.cells[self.cy].remove(self.cx);
                    self.cells[self.cy].push(BLANK);
                }
            }
            '@' => {
                for _ in 0..arg(params, 0, 1).min(cols - self.cx) {
                    self.cells[self.cy].insert(self.cx, BLANK);
                    self.cells[self.cy].pop();
                }
            }
            'S' => self.scroll_up(arg(params, 0, 1)),
            'T' => self.scroll_down(arg(params, 0, 1)),
            'r' => {
                let t = arg(params, 0, 1) - 1;
                let b = arg(params, 1, rows) - 1;
                if t < b && b < rows {
                    self.top = t;
                    self.bot = b;
                }
                self.cx = 0;
                self.cy = 0;
            }
            's' => self.saved = (self.cx, self.cy),
            'u' => (self.cx, self.cy) = self.saved,
            'm' => {
                if params.is_empty() {
                    self.rev = false;
                }
                for p in params.iter() {
                    match p.first().copied().unwrap_or(0) {
                        0 | 27 => self.rev = false,
                        7 => self.rev = true,
                        _ => {}
                    }
                }
            }
            'n' => {
                if arg(params, 0, 0) == 6 {
                    let r = format!("\x1b[{};{}R", self.cy + 1, self.cx + 1);
                    self.responses.extend_from_slice(r.as_bytes());
                } else if arg(params, 0, 0) == 5 {
                    self.responses.extend_from_slice(b"\x1b[0n");
                }
            }
            'c' if !private => self.responses.extend_from_slice(b"\x1b[?1;2c"),
            'h' | 'l' if private => {
                let on = action == 'h';
                for p in params.iter() {
                    match p.first().copied().unwrap_or(0) {
                        25 => self.cursor_visible = on,
                        47 | 1047 | 1049 => {
                            if on && self.alt.is_none() {
                                if p.first() == Some(&1049) {
                                    self.saved = (self.cx, self.cy);
                                }
                                let blank = vec![vec![BLANK; cols]; rows];
                                self.alt = Some(std::mem::replace(&mut self.cells, blank));
                            } else if !on {
                                if let Some(main) = self.alt.take() {
                                    self.cells = main;
                                    if p.first() == Some(&1049) {
                                        (self.cx, self.cy) = self.saved;
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
        self.cx = self.cx.min(cols - 1);
        self.cy = self.cy.min(rows - 1);
    }

    fn esc_dispatch(&mut self, inter: &[u8], _ignore: bool, b: u8) {
        if !inter.is_empty() {
            return; // charset selection etc.
        }
        match b {
            b'7' => self.saved = (self.cx, self.cy),
            b'8' => (self.cx, self.cy) = self.saved,
            b'D' => self.linefeed(),
            b'E' => {
                self.cx = 0;
                self.linefeed();
            }
            b'M' => {
                if self.cy == self.top {
                    self.scroll_down(1);
                } else {
                    self.cy = self.cy.saturating_sub(1);
                }
            }
            b'c' => *self = Grid::new(self.cols, self.rows),
            _ => {}
        }
    }
}

pub struct Terminal {
    /// Shell command to run instead of an interactive shell (e.g. the w3m browser).
    command: Option<String>,
    cwd: String,
    label: &'static str,
    key: &'static str,
    fd: Option<RawFd>,
    pid: libc::pid_t,
    grid: Option<Grid>,
    parser: vte::Parser,
    fs: u16,
    pending: bool,
    last_out: Instant,
    exited: bool,
    /// After fast (black/white) updates, do one quality refresh to clean up ghosting.
    cleanup_due: Option<Instant>,
}

const ID_SMALLER: u32 = 1;
const ID_BIGGER: u32 = 2;
const ID_KB: u32 = 3;
const ID_CTRLC: u32 = 4;

impl Terminal {
    pub fn new() -> Terminal {
        Terminal::with(None, "/root", "Terminal", "terminal")
    }

    /// Run `command` (via sh -c) in `cwd`, as its own resumable app.
    pub fn with(command: Option<String>, cwd: &str, label: &'static str, key: &'static str) -> Terminal {
        Terminal { command, cwd: cwd.into(), label, key, fd: None, pid: 0, grid: None, parser: vte::Parser::new(), fs: 18, pending: false,
                   last_out: Instant::now(), exited: false, cleanup_due: None }
    }

    fn spawn(&mut self, cols: usize, rows: usize) {
        let mut ws = libc::winsize { ws_row: rows as u16, ws_col: cols as u16, ws_xpixel: 0, ws_ypixel: 0 };
        let mut master: libc::c_int = -1;
        let pid = unsafe { libc::forkpty(&mut master, std::ptr::null_mut(), std::ptr::null_mut(), &mut ws) };
        if pid == 0 {
            // child
            let env = [("TERM", "vt100".to_string()), ("HOME", "/root".into()), ("LANG", "C.UTF-8".into()),
                       ("PS1", "kindle:\\w\\$ ".into()), ("COLUMNS", cols.to_string()), ("LINES", rows.to_string()),
                       ("PATH", "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin".into())];
            for (k, v) in env {
                std::env::set_var(k, v);
            }
            let _ = std::env::set_current_dir(&self.cwd).or_else(|_| std::env::set_current_dir("/"));
            let sh = std::ffi::CString::new("/bin/sh").unwrap();
            unsafe {
                match &self.command {
                    Some(cmd) => {
                        let c = std::ffi::CString::new("-c").unwrap();
                        let script = std::ffi::CString::new(cmd.as_str()).unwrap();
                        libc::execl(sh.as_ptr(), sh.as_ptr(), c.as_ptr(), script.as_ptr(), std::ptr::null::<libc::c_char>());
                    }
                    None => {
                        let l = std::ffi::CString::new("-l").unwrap();
                        libc::execl(sh.as_ptr(), sh.as_ptr(), l.as_ptr(), std::ptr::null::<libc::c_char>());
                    }
                }
                libc::_exit(127);
            }
        }
        if pid < 0 {
            self.exited = true;
            return;
        }
        unsafe {
            let fl = libc::fcntl(master, libc::F_GETFL);
            libc::fcntl(master, libc::F_SETFL, fl | libc::O_NONBLOCK);
        }
        self.fd = Some(master);
        self.pid = pid;
        self.grid = Some(Grid::new(cols, rows));
        self.parser = vte::Parser::new();
        self.exited = false;
    }

    fn write(&mut self, data: &[u8]) {
        if let Some(fd) = self.fd {
            unsafe { libc::write(fd, data.as_ptr() as *const libc::c_void, data.len()) };
        }
    }

    fn set_winsize(&self, cols: usize, rows: usize) {
        if let Some(fd) = self.fd {
            let ws = libc::winsize { ws_row: rows as u16, ws_col: cols as u16, ws_xpixel: 0, ws_ypixel: 0 };
            unsafe { libc::ioctl(fd, libc::TIOCSWINSZ as _, &ws) };
        }
    }

    fn close(&mut self) {
        if let Some(fd) = self.fd.take() {
            unsafe {
                libc::close(fd);
                if self.pid > 0 {
                    libc::kill(self.pid, libc::SIGHUP);
                    libc::waitpid(self.pid, std::ptr::null_mut(), libc::WNOHANG);
                }
            }
        }
        self.exited = true;
    }

    /// Feed pending pty output into the emulator (used by the simulator too).
    pub fn pump(&mut self) -> bool {
        let Some(fd) = self.fd else { return false };
        let mut buf = [0u8; 8192];
        let mut got = false;
        loop {
            let n = unsafe { libc::read(fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len()) };
            if n > 0 {
                got = true;
                if let Some(g) = self.grid.as_mut() {
                    for &b in &buf[..n as usize] {
                        self.parser.advance(g, b);
                    }
                }
            } else if n == 0 {
                self.close();
                break;
            } else {
                let e = std::io::Error::last_os_error();
                if e.kind() != std::io::ErrorKind::WouldBlock && e.kind() != std::io::ErrorKind::Interrupted {
                    self.close();
                }
                break;
            }
        }
        if let Some(g) = self.grid.as_mut() {
            if !g.responses.is_empty() {
                let r = std::mem::take(&mut g.responses);
                self.write(&r);
            }
        }
        if got {
            self.pending = true;
            self.last_out = Instant::now();
        }
        got
    }
}

impl App for Terminal {
    fn title(&self) -> String {
        self.label.into()
    }

    fn keyboard(&self) -> Option<bool> {
        Some(true)
    }

    fn persistent_key(&self) -> Option<&'static str> {
        if self.exited { None } else { Some(self.key) }
    }

    fn render(&mut self, cx: &mut Ctx, a: Rect, _sys: &mut Sys) {
        let cw = cx.f.mono_advance(self.fs);
        let lh = self.fs as i32 + 5;
        let cols = (((a.w() - 12) as f32 / cw) as usize).max(20);
        let rows = (((a.h() - 50) / lh) as usize).max(5);
        if self.grid.is_none() && !self.exited {
            self.spawn(cols, rows);
        } else if let Some(g) = self.grid.as_mut() {
            if (g.cols, g.rows) != (cols, rows) {
                g.resize(cols, rows);
                self.set_winsize(cols, rows);
            }
        }
        let b = Btn::size(18);
        cx.button(Rect::new(a.x0 + 8, a.y0 + 6, a.x0 + 70, a.y0 + 42), "A−", Some(Hit::App(ID_SMALLER)), b);
        cx.button(Rect::new(a.x0 + 76, a.y0 + 6, a.x0 + 138, a.y0 + 42), "A+", Some(Hit::App(ID_BIGGER)), b);
        cx.button(Rect::new(a.x0 + 144, a.y0 + 6, a.x0 + 280, a.y0 + 42), "Keyboard", Some(Hit::App(ID_KB)), b);
        cx.button(Rect::new(a.x0 + 286, a.y0 + 6, a.x0 + 380, a.y0 + 42), "Ctrl-C", Some(Hit::App(ID_CTRLC)), b);
        cx.text(a.x1 - 10, a.y0 + 24, &format!("{cols}×{rows}"), 15, DARK, Anchor::RM);
        let (tx, ty) = (a.x0 + 6, a.y0 + 48);
        if let Some(g) = &self.grid {
            for r in 0..g.rows {
                let y = ty + r as i32 * lh;
                let (text, rev) = g.row_text(r);
                let chars: Vec<char> = text.chars().collect();
                // reverse-video runs
                let mut c = 0;
                while c < chars.len() {
                    if rev[c] {
                        let s = c;
                        while c < chars.len() && rev[c] {
                            c += 1;
                        }
                        cx.c.fill(Rect::new(tx + (s as f32 * cw) as i32, y, tx + (c as f32 * cw) as i32, y + lh), BLACK);
                        let run: String = chars[s..c].iter().collect();
                        cx.c.text(cx.f, tx + (s as f32 * cw) as i32, y, &run, Face::Mono, self.fs, WHITE, Anchor::LT);
                    } else {
                        c += 1;
                    }
                }
                let plain: String = chars.iter().zip(&rev).map(|(&ch, &rv)| if rv { ' ' } else { ch }).collect();
                if !plain.trim().is_empty() {
                    cx.mono(tx, y, plain.trim_end(), self.fs, BLACK);
                }
            }
            if !self.exited && g.cursor_visible {
                let (ccx, ccy) = g.cursor();
                let x = tx + (ccx as f32 * cw) as i32;
                let y = ty + ccy as i32 * lh;
                cx.c.fill(Rect::new(x, y + lh - 4, x + cw as i32, y + lh - 1), BLACK);
            }
        }
        if self.exited {
            cx.text(a.cx(), a.y1 - 20, "Exited — press ⏎ to restart", 18, BLACK, Anchor::MM);
        }
    }

    fn action(&mut self, id: u32, _long: bool, sys: &mut Sys) -> Cmd {
        match id {
            ID_SMALLER => self.fs = (self.fs - 2).max(10),
            ID_BIGGER => self.fs = (self.fs + 2).min(28),
            ID_KB => sys.keyboard_visible = !sys.keyboard_visible,
            ID_CTRLC => self.write(&[3]),
            _ => {}
        }
        Cmd::Full
    }

    fn key(&mut self, k: Key, _sys: &mut Sys) -> Cmd {
        if self.exited {
            if k == Key::Enter {
                self.grid = None;
                self.exited = false;
                return Cmd::Full;
            }
            return Cmd::None;
        }
        let data: Vec<u8> = match k {
            Key::Char(c) => c.to_string().into_bytes(),
            Key::Enter => b"\r".to_vec(),
            Key::Backspace => vec![0x7f],
            Key::Tab => b"\t".to_vec(),
            Key::Esc => vec![0x1b],
            Key::Up => b"\x1b[A".to_vec(),
            Key::Down => b"\x1b[B".to_vec(),
            Key::Right => b"\x1b[C".to_vec(),
            Key::Left => b"\x1b[D".to_vec(),
            Key::Ctrl(c) => vec![(c as u8) & 0x1f],
        };
        self.write(&data);
        self.cleanup_due = Some(Instant::now() + std::time::Duration::from_millis(1500));
        Cmd::None
    }

    fn fd(&self) -> Option<RawFd> {
        self.fd
    }

    fn on_fd(&mut self, _sys: &mut Sys) -> Cmd {
        self.pump();
        if self.exited { Cmd::Fast } else { Cmd::None }
    }

    fn busy(&self) -> bool {
        self.pending || self.cleanup_due.is_some()
    }

    fn poll(&mut self, _sys: &mut Sys) -> Cmd {
        if self.pending && self.last_out.elapsed().as_millis() > 250 {
            self.pending = false;
            self.cleanup_due = Some(Instant::now() + std::time::Duration::from_millis(1500));
            return Cmd::Fast;
        }
        if let Some(t) = self.cleanup_due {
            if Instant::now() >= t && !self.pending {
                self.cleanup_due = None;
                return Cmd::Redraw;
            }
        }
        Cmd::None
    }

    fn on_hide(&mut self, sys: &mut Sys) {
        sys.keyboard_visible = true;
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        self.close();
    }
}
