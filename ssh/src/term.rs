// The screen: a grid of cells drawn each frame, and a renderer that sends only the cells that
// changed since the last frame. Colours are 24-bit; every terminal people are likely to ssh
// from handles them.
//
// Pictures (the planet, the pixel words) are drawn on a Canvas of "subpixels": each cell is
// split into a top and bottom half with ▀ and ▄, and since a cell is about twice as tall as
// it is wide, each half is close to square.

use std::fmt::Write as _;
use unicode_width::UnicodeWidthChar;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    pub const fn hex(h: u32) -> Rgb {
        Rgb((h >> 16) as u8, (h >> 8) as u8, h as u8)
    }
    /// From 0..1 floats, as the planet's shader works in.
    pub fn f(r: f32, g: f32, b: f32) -> Rgb {
        let c = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
        Rgb(c(r), c(g), c(b))
    }
    pub fn lerp(self, o: Rgb, t: f32) -> Rgb {
        let t = t.clamp(0.0, 1.0);
        let m = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t + 0.5) as u8;
        Rgb(m(self.0, o.0), m(self.1, o.1), m(self.2, o.2))
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Cell {
    /// '\0' marks the second half of a wide character, which the renderer skips.
    pub ch: char,
    pub fg: Rgb,
    pub bg: Rgb,
    pub bold: bool,
    pub ul: bool,
    /// An index into the buffer's links, plus one; 0 is no link.
    pub link: u16,
}

#[derive(Clone, Copy, Debug)]
pub struct Style {
    pub fg: Rgb,
    pub bold: bool,
    pub ul: bool,
    pub link: u16,
}

impl Style {
    pub fn fg(fg: Rgb) -> Style {
        Style { fg, bold: false, ul: false, link: 0 }
    }
}

/// A rectangle of cells: x, y, width, height.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
    }
}

#[derive(Clone)]
pub struct Buffer {
    pub w: i32,
    pub h: i32,
    pub bg: Rgb,
    cells: Vec<Cell>,
    links: Vec<String>,
}

impl Buffer {
    pub fn new(w: i32, h: i32, bg: Rgb) -> Buffer {
        let blank = Cell { ch: ' ', fg: bg, bg, bold: false, ul: false, link: 0 };
        Buffer { w, h, bg, cells: vec![blank; (w.max(0) * h.max(0)) as usize], links: Vec::new() }
    }

    pub fn get(&self, x: i32, y: i32) -> Option<&Cell> {
        (x >= 0 && y >= 0 && x < self.w && y < self.h).then(|| &self.cells[(y * self.w + x) as usize])
    }

    pub fn set(&mut self, x: i32, y: i32, c: Cell) {
        if x >= 0 && y >= 0 && x < self.w && y < self.h {
            self.cells[(y * self.w + x) as usize] = c;
        }
    }

    /// A link id for a URL, for Style::link.
    pub fn link(&mut self, url: &str) -> u16 {
        if let Some(i) = self.links.iter().position(|l| l == url) {
            return i as u16 + 1;
        }
        self.links.push(url.to_string());
        self.links.len() as u16
    }

    /// Writes text at x, y, inside clip. Returns the x just past it.
    pub fn text(&mut self, mut x: i32, y: i32, s: &str, st: Style, clip: Rect) -> i32 {
        for ch in s.chars() {
            let cw = ch.width().unwrap_or(0) as i32;
            if cw == 0 {
                continue;
            }
            if clip.contains(x, y) && clip.contains(x + cw - 1, y) {
                let bg = self.get(x, y).map_or(self.bg, |c| c.bg);
                self.set(x, y, Cell { ch, fg: st.fg, bg, bold: st.bold, ul: st.ul, link: st.link });
                if cw == 2 {
                    self.set(x + 1, y, Cell { ch: '\0', fg: st.fg, bg, bold: st.bold, ul: st.ul, link: st.link });
                }
            }
            x += cw;
        }
        x
    }

    /// Clears a run of cells back to the background, so text can sit over a picture.
    pub fn clear(&mut self, x: i32, y: i32, w: i32) {
        for i in x..x + w {
            self.set(i, y, Cell { ch: ' ', fg: self.bg, bg: self.bg, bold: false, ul: false, link: 0 });
        }
    }
}

/// Display width of a string, in cells.
pub fn width(s: &str) -> i32 {
    s.chars().map(|c| c.width().unwrap_or(0) as i32).sum()
}

/// A picture in subpixels: w across, h down, two per cell vertically.
pub struct Canvas {
    pub w: i32,
    pub h: i32,
    px: Vec<Option<Rgb>>,
}

impl Canvas {
    pub fn new(w: i32, h: i32) -> Canvas {
        Canvas { w, h, px: vec![None; (w.max(0) * h.max(0)) as usize] }
    }
    pub fn get(&self, x: i32, y: i32) -> Option<Rgb> {
        if x >= 0 && y >= 0 && x < self.w && y < self.h { self.px[(y * self.w + x) as usize] } else { None }
    }
    pub fn set(&mut self, x: i32, y: i32, c: Rgb) {
        if x >= 0 && y >= 0 && x < self.w && y < self.h {
            self.px[(y * self.w + x) as usize] = Some(c);
        }
    }
    pub fn fill(&mut self, x: i32, y: i32, w: i32, h: i32, c: Rgb) {
        for j in y..y + h {
            for i in x..x + w {
                self.set(i, j, c);
            }
        }
    }

    /// Draws the canvas with its top-left at cell (x, y), inside clip. Empty subpixels show
    /// what's already there. `fade` blends every colour toward the background (0 is gone).
    pub fn blit(&self, buf: &mut Buffer, x: i32, y: i32, clip: Rect, fade: f32) {
        let bg = buf.bg;
        let f = |c: Rgb| if fade >= 1.0 { c } else { bg.lerp(c, fade) };
        for row in 0..(self.h + 1) / 2 {
            for col in 0..self.w {
                let (cx, cy) = (x + col, y + row);
                if !clip.contains(cx, cy) {
                    continue;
                }
                let top = self.get(col, row * 2).map(f);
                let bot = self.get(col, row * 2 + 1).map(f);
                let under = buf.get(cx, cy).map_or(bg, |c| c.bg);
                let cell = |ch, fg, bg| Cell { ch, fg, bg, bold: false, ul: false, link: 0 };
                let c = match (top, bot) {
                    (None, None) => continue,
                    (Some(t), Some(b)) if t == b => cell(' ', t, t),
                    (Some(t), Some(b)) => cell('▀', t, b),
                    (Some(t), None) => cell('▀', t, under),
                    (None, Some(b)) => cell('▄', b, under),
                };
                buf.set(cx, cy, c);
            }
        }
    }
}

/// A picture in braille dots: each cell is 2 dots across and 4 down, so a dot is about
/// square, and they have gaps between them like the site's planet. A cell has one colour,
/// though, so each dot carries a rank: where dots share a cell, only the highest-ranked ones
/// show, in the brightest of their colours (a lit square doesn't tint its dim neighbours).
pub struct Dots {
    pub w: i32,
    pub h: i32,
    px: Vec<Option<(Rgb, u8)>>,
}

impl Dots {
    pub fn new(w: i32, h: i32) -> Dots {
        Dots { w, h, px: vec![None; (w.max(0) * h.max(0)) as usize] }
    }

    pub fn set(&mut self, x: i32, y: i32, c: Rgb, rank: u8) {
        if x >= 0 && y >= 0 && x < self.w && y < self.h {
            self.px[(y * self.w + x) as usize] = Some((c, rank));
        }
    }

    /// Draws the dots with their top-left at cell (0, 0), inside clip, over what's there.
    pub fn blit(&self, buf: &mut Buffer, clip: Rect) {
        // the bit for each dot in a braille character, by column then row
        const BITS: [[u32; 4]; 2] = [[0x01, 0x02, 0x04, 0x40], [0x08, 0x10, 0x20, 0x80]];
        let lum = |c: Rgb| c.0 as u32 * 3 + c.1 as u32 * 6 + c.2 as u32;
        for cy in 0..(self.h + 3) / 4 {
            for cx in 0..(self.w + 1) / 2 {
                if !clip.contains(cx, cy) {
                    continue;
                }
                let mut dots: [(u32, Option<(Rgb, u8)>); 8] = [(0, None); 8];
                for (i, (dx, dy)) in (0..2).flat_map(|dx| (0..4).map(move |dy| (dx, dy))).enumerate() {
                    let (x, y) = (cx * 2 + dx, cy * 4 + dy);
                    let p = if x < self.w && y < self.h { self.px[(y * self.w + x) as usize] } else { None };
                    dots[i] = (BITS[dx as usize][dy as usize], p);
                }
                let Some(top) = dots.iter().filter_map(|d| d.1.map(|p| p.1)).max() else { continue };
                let mut bits = 0;
                let mut fg: Option<Rgb> = None;
                for (bit, p) in dots {
                    if let Some((c, rank)) = p
                        && rank == top
                    {
                        bits |= bit;
                        if fg.is_none_or(|f| lum(c) > lum(f)) {
                            fg = Some(c);
                        }
                    }
                }
                let bg = buf.get(cx, cy).map_or(buf.bg, |c| c.bg);
                let ch = char::from_u32(0x2800 + bits).unwrap_or(' ');
                buf.set(cx, cy, Cell { ch, fg: fg.unwrap_or(bg), bg, bold: false, ul: false, link: 0 });
            }
        }
    }
}

/// Turns frames into escape codes, sending only what changed since the last one.
pub struct Screen {
    prev: Option<Buffer>,
}

impl Screen {
    pub fn new() -> Screen {
        Screen { prev: None }
    }

    /// Forget the last frame, so the next one is drawn in full (after a resize).
    pub fn invalidate(&mut self) {
        self.prev = None;
    }

    pub fn draw(&mut self, buf: &Buffer, out: &mut Vec<u8>) {
        let full = self.prev.as_ref().is_none_or(|p| p.w != buf.w || p.h != buf.h);
        let mut s = String::new();
        // where the cursor is, and what it's drawing with; None when unknown
        let mut at: Option<(i32, i32)> = None;
        let mut pen: Option<(Rgb, Rgb, bool, bool)> = None;
        let mut link: u16 = 0;
        if full {
            // a clean slate in the background colour
            let b = buf.bg;
            let _ = write!(s, "\x1b[0m\x1b[48;2;{};{};{}m\x1b[2J", b.0, b.1, b.2);
        }
        for y in 0..buf.h {
            for x in 0..buf.w {
                let c = buf.get(x, y).unwrap();
                if c.ch == '\0' {
                    continue;
                }
                if !full {
                    let p = self.prev.as_ref().unwrap();
                    let same_link = |a: u16, b: u16| match (a, b) {
                        (0, 0) => true,
                        (0, _) | (_, 0) => false,
                        (a, b) => p.links[a as usize - 1] == buf.links[b as usize - 1],
                    };
                    if let Some(o) = p.get(x, y)
                        && (Cell { link: c.link, ..*o }) == *c
                        && same_link(o.link, c.link)
                    {
                        continue;
                    }
                }
                if at != Some((x, y)) {
                    let _ = write!(s, "\x1b[{};{}H", y + 1, x + 1);
                }
                let want = (c.fg, c.bg, c.bold, c.ul);
                if pen != Some(want) {
                    match pen {
                        Some((_, _, b, u)) if b == c.bold && u == c.ul => {
                            s.push_str("\x1b[");
                            let mut sep = "";
                            if pen.unwrap().0 != c.fg {
                                let _ = write!(s, "38;2;{};{};{}", c.fg.0, c.fg.1, c.fg.2);
                                sep = ";";
                            }
                            if pen.unwrap().1 != c.bg {
                                let _ = write!(s, "{sep}48;2;{};{};{}", c.bg.0, c.bg.1, c.bg.2);
                            }
                            s.push('m');
                        }
                        _ => {
                            let _ = write!(
                                s,
                                "\x1b[0{}{};38;2;{};{};{};48;2;{};{};{}m",
                                if c.bold { ";1" } else { "" },
                                if c.ul { ";4" } else { "" },
                                c.fg.0, c.fg.1, c.fg.2, c.bg.0, c.bg.1, c.bg.2
                            );
                        }
                    }
                    pen = Some(want);
                }
                if c.link != link {
                    let url = if c.link == 0 { "" } else { &buf.links[c.link as usize - 1] };
                    let _ = write!(s, "\x1b]8;;{url}\x1b\\");
                    link = c.link;
                }
                s.push(c.ch);
                let cw = c.ch.width().unwrap_or(1) as i32;
                at = Some((x + cw, y));
            }
        }
        if link != 0 {
            s.push_str("\x1b]8;;\x1b\\");
        }
        if !s.is_empty() {
            // drawn as one update, where the terminal supports it, so it never shows half a frame
            out.extend_from_slice(b"\x1b[?2026h");
            out.extend_from_slice(s.as_bytes());
            out.extend_from_slice(b"\x1b[?2026l");
        }
        self.prev = Some(buf.clone());
    }
}
