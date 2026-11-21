//! Games suited to e-ink (turn-based, no animation): Sudoku and Chess.

use crate::canvas::*;
use crate::sys::Sys;
use crate::ui::*;
use std::sync::{Arc, Mutex};
use std::time::Instant;

struct Rng(u64);

impl Rng {
    fn new() -> Rng {
        let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |d| d.as_nanos() as u64);
        Rng(t | 1)
    }
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
    fn shuffle<T>(&mut self, v: &mut [T]) {
        for i in (1..v.len()).rev() {
            v.swap(i, self.below(i + 1));
        }
    }
}

// --- menu -------------------------------------------------------------------------------------------
pub struct GamesApp;

impl App for GamesApp {
    fn title(&self) -> String {
        "Games".into()
    }

    fn render(&mut self, cx: &mut Ctx, a: Rect, _sys: &mut Sys) {
        let items = [("Sudoku", "Classic number puzzle · 3 levels"), ("Chess", "Play white against a small engine")];
        let mut y = a.y0 + 30;
        for (i, (t, sub)) in items.iter().enumerate() {
            cx.button(Rect::new(a.x0 + 20, y, a.x1 - 20, y + 120), t, Some(Hit::App(i as u32)),
                      Btn { size: 30, bold: true, sub: Some(sub), ..Default::default() });
            y += 140;
        }
    }

    fn action(&mut self, id: u32, _long: bool, _sys: &mut Sys) -> Cmd {
        match id {
            0 => Cmd::Push(Box::new(Sudoku::new(1))),
            1 => Cmd::Push(Box::new(Chess::new())),
            _ => Cmd::None,
        }
    }
}

// --- Sudoku -----------------------------------------------------------------------------------------
fn sudoku_ok(g: &[u8; 81], i: usize, v: u8) -> bool {
    let (r, c) = (i / 9, i % 9);
    for k in 0..9 {
        if g[r * 9 + k] == v || g[k * 9 + c] == v {
            return false;
        }
    }
    let (br, bc) = (r / 3 * 3, c / 3 * 3);
    for dr in 0..3 {
        for dc in 0..3 {
            if g[(br + dr) * 9 + bc + dc] == v {
                return false;
            }
        }
    }
    true
}

fn sudoku_fill(g: &mut [u8; 81], rng: &mut Rng) -> bool {
    let Some(i) = g.iter().position(|&v| v == 0) else { return true };
    let mut vals: Vec<u8> = (1..=9).collect();
    rng.shuffle(&mut vals);
    for v in vals {
        if sudoku_ok(g, i, v) {
            g[i] = v;
            if sudoku_fill(g, rng) {
                return true;
            }
            g[i] = 0;
        }
    }
    false
}

fn sudoku_count(g: &mut [u8; 81], limit: u32) -> u32 {
    let Some(i) = g.iter().position(|&v| v == 0) else { return 1 };
    let mut n = 0;
    for v in 1..=9 {
        if sudoku_ok(g, i, v) {
            g[i] = v;
            n += sudoku_count(g, limit - n);
            g[i] = 0;
            if n >= limit {
                break;
            }
        }
    }
    n
}

pub struct Sudoku {
    solution: [u8; 81],
    given: [bool; 81],
    cells: [u8; 81],
    sel: Option<usize>,
    wrong: [bool; 81],
    level: usize,
    started: Instant,
    solved: Option<u64>,
}

const LEVELS: [(&str, usize); 3] = [("Easy", 38), ("Medium", 46), ("Hard", 54)];
const SU_NEW: u32 = 100;
const SU_CHECK: u32 = 101;
const SU_ERASE: u32 = 102;
const SU_NUM: u32 = 200;
const T_LEVEL: u32 = 1;

impl Sudoku {
    pub fn new(level: usize) -> Sudoku {
        let mut rng = Rng::new();
        let mut sol = [0u8; 81];
        sudoku_fill(&mut sol, &mut rng);
        let mut puzzle = sol;
        let mut order: Vec<usize> = (0..81).collect();
        rng.shuffle(&mut order);
        let mut removed = 0;
        for i in order {
            if removed >= LEVELS[level].1 {
                break;
            }
            let keep = puzzle[i];
            puzzle[i] = 0;
            let mut test = puzzle;
            if sudoku_count(&mut test, 2) == 1 {
                removed += 1;
            } else {
                puzzle[i] = keep;
            }
        }
        let mut given = [false; 81];
        for i in 0..81 {
            given[i] = puzzle[i] != 0;
        }
        Sudoku { solution: sol, given, cells: puzzle, sel: None, wrong: [false; 81], level, started: Instant::now(), solved: None }
    }
}

impl App for Sudoku {
    fn title(&self) -> String {
        format!("Sudoku · {}", LEVELS[self.level].0)
    }

    fn render(&mut self, cx: &mut Ctx, a: Rect, _sys: &mut Sys) {
        let cell = 60;
        let (gx, gy) = (a.x0 + (a.w() - cell * 9) / 2, a.y0 + 12);
        let selv = self.sel.map(|i| self.cells[i]).filter(|&v| v != 0);
        for i in 0..81 {
            let (r, c) = ((i / 9) as i32, (i % 9) as i32);
            let rect = Rect::new(gx + c * cell, gy + r * cell, gx + (c + 1) * cell, gy + (r + 1) * cell);
            if Some(i) == self.sel {
                cx.c.fill(rect, LIGHT);
            } else if selv.is_some() && self.cells[i] == selv.unwrap() {
                cx.c.fill(rect.inset(6), 235);
            }
            if self.wrong[i] {
                cx.c.fill(rect.inset(4), BLACK);
            }
            let v = self.cells[i];
            if v != 0 {
                let color = if self.wrong[i] { WHITE } else { BLACK };
                let face = if self.given[i] { Face::Bold } else { Face::Sans };
                cx.c.text(cx.f, rect.cx(), rect.cy(), &v.to_string(), face, 32, color, Anchor::MM);
            }
            if !self.given[i] {
                cx.hit(rect, Hit::App(i as u32), None, &format!("cell{i}"));
            }
        }
        for k in 0..=9 {
            let w = if k % 3 == 0 { 3 } else { 1 };
            cx.c.fill(Rect::new(gx + k * cell - w / 2, gy, gx + k * cell - w / 2 + w, gy + 9 * cell), BLACK);
            cx.c.fill(Rect::new(gx, gy + k * cell - w / 2, gx + 9 * cell, gy + k * cell - w / 2 + w), BLACK);
        }
        // number pad
        let py = gy + 9 * cell + 16;
        let bw = (a.w() - 20) / 9;
        for n in 1..=9u32 {
            let x = a.x0 + 10 + (n as i32 - 1) * bw;
            cx.button(Rect::new(x + 2, py, x + bw - 2, py + 64), &n.to_string(), Some(Hit::App(SU_NUM + n)),
                      Btn { size: 30, bold: true, radius: 8, ..Default::default() });
        }
        let by = py + 76;
        let w3 = (a.w() - 40) / 3;
        cx.button(Rect::new(a.x0 + 10, by, a.x0 + 10 + w3, by + 56), "Erase", Some(Hit::App(SU_ERASE)), Btn::size(20));
        cx.button(Rect::new(a.x0 + 20 + w3, by, a.x0 + 20 + 2 * w3, by + 56), "Check", Some(Hit::App(SU_CHECK)), Btn::size(20));
        cx.button(Rect::new(a.x0 + 30 + 2 * w3, by, a.x1 - 10, by + 56), "New game", Some(Hit::App(SU_NEW)), Btn::size(20));
        let secs = self.solved.unwrap_or_else(|| self.started.elapsed().as_secs());
        let left = self.cells.iter().filter(|&&v| v == 0).count();
        let status = if self.solved.is_some() { format!("Solved in {}:{:02}! 🎉", secs / 60, secs % 60) }
                     else { format!("{left} cells left · {}:{:02}", secs / 60, secs % 60) };
        cx.text(a.cx(), by + 82, &status, 18, DARK, Anchor::MM);
    }

    fn action(&mut self, id: u32, _long: bool, _sys: &mut Sys) -> Cmd {
        match id {
            i if i < 81 => {
                self.sel = Some(i as usize);
                Cmd::Fast
            }
            SU_ERASE => {
                if let Some(i) = self.sel {
                    self.cells[i] = 0;
                    self.wrong[i] = false;
                }
                Cmd::Fast
            }
            SU_CHECK => {
                let mut bad = 0;
                for i in 0..81 {
                    self.wrong[i] = self.cells[i] != 0 && self.cells[i] != self.solution[i];
                    bad += self.wrong[i] as usize;
                }
                Cmd::Batch(vec![Cmd::Redraw, Cmd::Toast(if bad == 0 { "No mistakes so far".into() } else { format!("{bad} wrong") })])
            }
            SU_NEW => Cmd::Dialog(Dialog::menu(T_LEVEL, "New game", LEVELS.iter().map(|l| l.0.to_string()).collect())),
            n if n > SU_NUM && n <= SU_NUM + 9 => {
                if let Some(i) = self.sel {
                    self.cells[i] = (n - SU_NUM) as u8;
                    self.wrong[i] = false;
                    if self.solved.is_none() && self.cells == self.solution {
                        self.solved = Some(self.started.elapsed().as_secs());
                        return Cmd::Full;
                    }
                }
                Cmd::Fast
            }
            _ => Cmd::None,
        }
    }

    fn dialog(&mut self, tag: u32, choice: usize, _t: String, _sys: &mut Sys) -> Cmd {
        if tag == T_LEVEL && choice < LEVELS.len() {
            *self = Sudoku::new(choice);
            return Cmd::Full;
        }
        Cmd::None
    }

    fn tick(&mut self, _sys: &mut Sys) -> Cmd {
        // update the timer once a minute (e-ink friendly)
        if self.solved.is_none() && self.started.elapsed().as_secs() % 60 == 0 { Cmd::Fast } else { Cmd::None }
    }
}

// --- Chess ------------------------------------------------------------------------------------------
const P: i8 = 1;
const N: i8 = 2;
const B: i8 = 3;
const R: i8 = 4;
const Q: i8 = 5;
const K: i8 = 6;

#[derive(Clone)]
pub struct Pos {
    b: [i8; 64], // row 0 = rank 8 (black's back rank); positive = white
    white: bool, // side to move
    castle: [bool; 4], // white K, white Q, black K, black Q
    ep: Option<usize>,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Mv {
    from: usize,
    to: usize,
}

impl Pos {
    fn start() -> Pos {
        let back = [R, N, B, Q, K, B, N, R];
        let mut b = [0i8; 64];
        for c in 0..8 {
            b[c] = -back[c];
            b[8 + c] = -P;
            b[48 + c] = P;
            b[56 + c] = back[c];
        }
        Pos { b, white: true, castle: [true; 4], ep: None }
    }

    fn own(&self, v: i8) -> bool {
        if self.white { v > 0 } else { v < 0 }
    }

    fn attacked(&self, sq: usize, by_white: bool) -> bool {
        let (r, c) = ((sq / 8) as i32, (sq % 8) as i32);
        let s = if by_white { 1 } else { -1 };
        let at = |rr: i32, cc: i32| -> i8 { if (0..8).contains(&rr) && (0..8).contains(&cc) { self.b[(rr * 8 + cc) as usize] } else { 0 } };
        // pawns
        let pr = if by_white { r + 1 } else { r - 1 };
        if at(pr, c - 1) == s * P || at(pr, c + 1) == s * P {
            return true;
        }
        for (dr, dc) in [(1, 2), (2, 1), (-1, 2), (-2, 1), (1, -2), (2, -1), (-1, -2), (-2, -1)] {
            if at(r + dr, c + dc) == s * N {
                return true;
            }
        }
        for dr in -1..=1 {
            for dc in -1..=1 {
                if (dr, dc) != (0, 0) && at(r + dr, c + dc) == s * K {
                    return true;
                }
            }
        }
        for (dr, dc, kinds) in [(1, 0, [R, Q]), (-1, 0, [R, Q]), (0, 1, [R, Q]), (0, -1, [R, Q]),
                                (1, 1, [B, Q]), (1, -1, [B, Q]), (-1, 1, [B, Q]), (-1, -1, [B, Q])] {
            let (mut rr, mut cc) = (r + dr, c + dc);
            while (0..8).contains(&rr) && (0..8).contains(&cc) {
                let v = self.b[(rr * 8 + cc) as usize];
                if v != 0 {
                    if kinds.contains(&(v * s)) {
                        return true;
                    }
                    break;
                }
                rr += dr;
                cc += dc;
            }
        }
        false
    }

    fn king(&self, white: bool) -> usize {
        let k = if white { K } else { -K };
        self.b.iter().position(|&v| v == k).unwrap_or(0)
    }

    fn in_check(&self, white: bool) -> bool {
        self.attacked(self.king(white), !white)
    }

    fn pseudo(&self) -> Vec<Mv> {
        let mut v = vec![];
        let s: i8 = if self.white { 1 } else { -1 };
        for sq in 0..64 {
            let p = self.b[sq];
            if p == 0 || !self.own(p) {
                continue;
            }
            let (r, c) = ((sq / 8) as i32, (sq % 8) as i32);
            let push = |v: &mut Vec<Mv>, rr: i32, cc: i32| {
                if (0..8).contains(&rr) && (0..8).contains(&cc) {
                    let t = self.b[(rr * 8 + cc) as usize];
                    if t == 0 || !self.own(t) {
                        v.push(Mv { from: sq, to: (rr * 8 + cc) as usize });
                    }
                }
            };
            match p.abs() {
                P => {
                    let dir = -s as i32; // white moves up (towards row 0)
                    let one = r + dir;
                    if (0..8).contains(&one) && self.b[(one * 8 + c) as usize] == 0 {
                        v.push(Mv { from: sq, to: (one * 8 + c) as usize });
                        let start = if self.white { 6 } else { 1 };
                        let two = r + 2 * dir;
                        if r == start && self.b[(two * 8 + c) as usize] == 0 {
                            v.push(Mv { from: sq, to: (two * 8 + c) as usize });
                        }
                    }
                    for dc in [-1, 1] {
                        let (rr, cc) = (one, c + dc);
                        if (0..8).contains(&rr) && (0..8).contains(&cc) {
                            let t = (rr * 8 + cc) as usize;
                            if (self.b[t] != 0 && !self.own(self.b[t])) || Some(t) == self.ep {
                                v.push(Mv { from: sq, to: t });
                            }
                        }
                    }
                }
                N => {
                    for (dr, dc) in [(1, 2), (2, 1), (-1, 2), (-2, 1), (1, -2), (2, -1), (-1, -2), (-2, -1)] {
                        push(&mut v, r + dr, c + dc);
                    }
                }
                K => {
                    for dr in -1..=1 {
                        for dc in -1..=1 {
                            if (dr, dc) != (0, 0) {
                                push(&mut v, r + dr, c + dc);
                            }
                        }
                    }
                    // castling (squares empty, king not in / through check)
                    let (home, ki, qi) = if self.white { (60, 0, 1) } else { (4, 2, 3) };
                    if sq == home && !self.in_check(self.white) {
                        if self.castle[ki] && self.b[home + 1] == 0 && self.b[home + 2] == 0
                            && !self.attacked(home + 1, !self.white) && self.b[home + 3] == s * R {
                            v.push(Mv { from: sq, to: home + 2 });
                        }
                        if self.castle[qi] && self.b[home - 1] == 0 && self.b[home - 2] == 0 && self.b[home - 3] == 0
                            && !self.attacked(home - 1, !self.white) && self.b[home - 4] == s * R {
                            v.push(Mv { from: sq, to: home - 2 });
                        }
                    }
                }
                kind => {
                    let dirs: &[(i32, i32)] = match kind {
                        B => &[(1, 1), (1, -1), (-1, 1), (-1, -1)],
                        R => &[(1, 0), (-1, 0), (0, 1), (0, -1)],
                        _ => &[(1, 1), (1, -1), (-1, 1), (-1, -1), (1, 0), (-1, 0), (0, 1), (0, -1)],
                    };
                    for &(dr, dc) in dirs {
                        let (mut rr, mut cc) = (r + dr, c + dc);
                        while (0..8).contains(&rr) && (0..8).contains(&cc) {
                            let t = self.b[(rr * 8 + cc) as usize];
                            if t == 0 {
                                v.push(Mv { from: sq, to: (rr * 8 + cc) as usize });
                            } else {
                                if !self.own(t) {
                                    v.push(Mv { from: sq, to: (rr * 8 + cc) as usize });
                                }
                                break;
                            }
                            rr += dr;
                            cc += dc;
                        }
                    }
                }
            }
        }
        v
    }

    fn make(&self, m: Mv) -> Pos {
        let mut n = self.clone();
        let p = n.b[m.from];
        // en passant capture
        if p.abs() == P && Some(m.to) == self.ep && n.b[m.to] == 0 {
            let cap = if p > 0 { m.to + 8 } else { m.to - 8 };
            n.b[cap] = 0;
        }
        n.b[m.to] = p;
        n.b[m.from] = 0;
        // promotion (always to a queen)
        if p.abs() == P && (m.to < 8 || m.to >= 56) {
            n.b[m.to] = if p > 0 { Q } else { -Q };
        }
        // castling: move the rook too
        if p.abs() == K && (m.to as i32 - m.from as i32).abs() == 2 {
            let (rf, rt) = if m.to > m.from { (m.from + 3, m.from + 1) } else { (m.from - 4, m.from - 1) };
            n.b[rt] = n.b[rf];
            n.b[rf] = 0;
        }
        n.ep = if p.abs() == P && (m.to as i32 - m.from as i32).abs() == 16 { Some((m.from + m.to) / 2) } else { None };
        for (sq, idx) in [(60usize, 0usize), (63, 0), (56, 1), (4, 2), (7, 2), (0, 3)] {
            if m.from == sq || m.to == sq {
                n.castle[idx] = false;
            }
        }
        if m.from == 60 || m.to == 60 {
            n.castle[1] = false;
        }
        if m.from == 4 || m.to == 4 {
            n.castle[3] = false;
        }
        n.white = !self.white;
        n
    }

    pub fn legal(&self) -> Vec<Mv> {
        self.pseudo().into_iter().filter(|&m| !self.make(m).in_check(self.white)).collect()
    }

    /// Material + a little positional sense, from white's point of view.
    fn eval(&self) -> i32 {
        let val = [0, 100, 320, 330, 500, 900, 0];
        let mut s = 0;
        for sq in 0..64 {
            let p = self.b[sq];
            if p == 0 {
                continue;
            }
            let (r, c) = ((sq / 8) as i32, (sq % 8) as i32);
            let center = 6 - ((2 * r - 7).abs() + (2 * c - 7).abs()) / 2; // 0..6, higher in the middle
            let mut v = val[p.unsigned_abs() as usize];
            match p.abs() {
                N | B => v += center * 4,
                P => v += if p > 0 { (6 - r) * 6 } else { (r - 1) * 6 } + center * 2,
                _ => {}
            }
            s += if p > 0 { v } else { -v };
        }
        s
    }

    fn search(&self, depth: u32, mut alpha: i32, beta: i32) -> i32 {
        if depth == 0 {
            let e = self.eval();
            return if self.white { e } else { -e };
        }
        let mut moves = self.legal();
        if moves.is_empty() {
            return if self.in_check(self.white) { -100_000 - depth as i32 } else { 0 };
        }
        moves.sort_by_key(|m| -(self.b[m.to].unsigned_abs() as i32));
        for m in moves {
            let score = -self.make(m).search(depth - 1, -beta, -alpha);
            if score >= beta {
                return beta;
            }
            alpha = alpha.max(score);
        }
        alpha
    }

    pub fn best(&self, depth: u32) -> Option<Mv> {
        let mut moves = self.legal();
        moves.sort_by_key(|m| -(self.b[m.to].unsigned_abs() as i32));
        let mut best = None;
        let mut alpha = -1_000_000;
        for m in moves {
            let score = -self.make(m).search(depth - 1, -1_000_000, -alpha);
            if best.is_none() || score > alpha {
                alpha = score;
                best = Some(m);
            }
        }
        best
    }
}

pub struct Chess {
    pos: Pos,
    history: Vec<Pos>,
    sel: Option<usize>,
    last: Option<Mv>,
    thinking: Option<Arc<Mutex<Option<Option<Mv>>>>>,
    status: String,
}

const CH_NEW: u32 = 100;
const CH_UNDO: u32 = 101;

impl Chess {
    pub fn new() -> Chess {
        Chess { pos: Pos::start(), history: vec![], sel: None, last: None, thinking: None, status: "Your move (white)".into() }
    }

    fn update_status(&mut self) {
        let moves = self.pos.legal();
        let check = self.pos.in_check(self.pos.white);
        self.status = match (moves.is_empty(), check, self.pos.white) {
            (true, true, true) => "Checkmate — the engine wins".into(),
            (true, true, false) => "Checkmate — you win! 🎉".into(),
            (true, false, _) => "Stalemate — draw".into(),
            (false, true, true) => "Check! Your move".into(),
            (false, _, true) => "Your move (white)".into(),
            _ => "Thinking…".into(),
        };
    }

    fn engine_move(&mut self) {
        let p = self.pos.clone();
        let slot = Arc::new(Mutex::new(None));
        let s2 = slot.clone();
        std::thread::spawn(move || {
            let m = p.best(3);
            *s2.lock().unwrap() = Some(m);
        });
        self.thinking = Some(slot);
        self.status = "Thinking…".into();
    }
}

fn glyph(p: i8) -> &'static str {
    match p {
        1 => "♙", 2 => "♘", 3 => "♗", 4 => "♖", 5 => "♕", 6 => "♔",
        -1 => "♟", -2 => "♞", -3 => "♝", -4 => "♜", -5 => "♛", -6 => "♚",
        _ => "",
    }
}

impl App for Chess {
    fn title(&self) -> String {
        "Chess".into()
    }

    fn render(&mut self, cx: &mut Ctx, a: Rect, _sys: &mut Sys) {
        let sq = 68;
        let (bx, by) = (a.x0 + (a.w() - sq * 8) / 2, a.y0 + 14);
        let targets: Vec<usize> = self.sel.map(|s| self.pos.legal().into_iter().filter(|m| m.from == s).map(|m| m.to).collect())
            .unwrap_or_default();
        for i in 0..64 {
            let (r, c) = ((i / 8) as i32, (i % 8) as i32);
            let rect = Rect::new(bx + c * sq, by + r * sq, bx + (c + 1) * sq, by + (r + 1) * sq);
            let dark = (r + c) % 2 == 1;
            cx.c.fill(rect, if dark { 175 } else { WHITE });
            if self.last.map_or(false, |m| m.from == i || m.to == i) {
                cx.c.outline(rect.inset(2), 3, DARK);
            }
            if Some(i) == self.sel {
                cx.c.outline(rect.inset(1), 5, BLACK);
            }
            let p = self.pos.b[i];
            if p != 0 {
                cx.c.text(cx.f, rect.cx(), rect.cy() + 2, glyph(p), Face::Sans, 54, BLACK, Anchor::MM);
            }
            if targets.contains(&i) {
                let d = Rect::new(rect.cx() - 8, rect.cy() - 8, rect.cx() + 8, rect.cy() + 8);
                cx.c.round_fill(d, 8, if p != 0 { BLACK } else { DARK });
            }
            cx.hit(rect, Hit::App(i as u32), None, &format!("sq{i}"));
        }
        cx.c.outline(Rect::new(bx - 2, by - 2, bx + 8 * sq + 2, by + 8 * sq + 2), 2, BLACK);
        let y = by + 8 * sq + 24;
        cx.bold(a.cx(), y, &self.status.clone(), 22, BLACK, Anchor::MM);
        let by2 = y + 30;
        let half = (a.w() - 36) / 2;
        cx.button(Rect::new(a.x0 + 12, by2, a.x0 + 12 + half, by2 + 56), "Undo", Some(Hit::App(CH_UNDO)),
                  Btn { disabled: self.history.is_empty() || self.thinking.is_some(), ..Btn::size(20) });
        cx.button(Rect::new(a.x1 - 12 - half, by2, a.x1 - 12, by2 + 56), "New game", Some(Hit::App(CH_NEW)), Btn::size(20));
    }

    fn busy(&self) -> bool {
        self.thinking.is_some()
    }

    fn poll(&mut self, _sys: &mut Sys) -> Cmd {
        let done = self.thinking.as_ref().and_then(|t| t.lock().unwrap().take());
        let Some(m) = done else { return Cmd::None };
        self.thinking = None;
        if let Some(m) = m {
            self.pos = self.pos.make(m);
            self.last = Some(m);
        }
        self.update_status();
        Cmd::Redraw
    }

    fn action(&mut self, id: u32, _long: bool, _sys: &mut Sys) -> Cmd {
        if self.thinking.is_some() && id < 64 {
            return Cmd::None;
        }
        match id {
            CH_NEW => {
                *self = Chess::new();
                Cmd::Full
            }
            CH_UNDO => {
                // take back the engine's reply and your move
                if self.history.len() >= 1 {
                    self.pos = self.history.pop().unwrap();
                    self.sel = None;
                    self.last = None;
                    self.update_status();
                }
                Cmd::Full
            }
            i if i < 64 => {
                let i = i as usize;
                if !self.pos.white {
                    return Cmd::None;
                }
                if let Some(s) = self.sel {
                    if let Some(m) = self.pos.legal().into_iter().find(|m| m.from == s && m.to == i) {
                        self.history.push(self.pos.clone());
                        self.pos = self.pos.make(m);
                        self.last = Some(m);
                        self.sel = None;
                        self.update_status();
                        if !self.pos.legal().is_empty() {
                            self.engine_move();
                        }
                        return Cmd::Redraw;
                    }
                }
                self.sel = if self.pos.b[i] > 0 { Some(i) } else { None };
                Cmd::Fast
            }
            _ => Cmd::None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chess_start_has_20_moves() {
        assert_eq!(Pos::start().legal().len(), 20);
    }

    #[test]
    fn chess_perft_depth2() {
        // 400 positions after one move each side from the start
        let p = Pos::start();
        let n: usize = p.legal().iter().map(|&m| p.make(m).legal().len()).sum();
        assert_eq!(n, 400);
    }

    #[test]
    fn chess_engine_takes_free_queen() {
        // white queen can capture an undefended black queen
        let mut p = Pos::start();
        p.b = [0; 64];
        p.b[60] = K;
        p.b[4] = -K;
        p.b[35] = Q; // d4
        p.b[27] = -Q; // d5, undefended
        p.castle = [false; 4];
        let m = p.best(3).unwrap();
        assert_eq!((m.from, m.to), (35, 27));
    }

    #[test]
    fn sudoku_has_unique_solution() {
        let s = Sudoku::new(1);
        let mut g = s.cells;
        assert_eq!(sudoku_count(&mut g, 2), 1);
        assert!(s.cells.iter().zip(s.solution.iter()).all(|(c, sol)| *c == 0 || c == sol));
    }
}
