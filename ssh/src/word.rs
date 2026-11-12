// A word as binary that rebuilds itself into pixel letters, as PixelWord.astro does on the
// site: the bytes sit in a grid, three to a column, and every square has a spot in the binary
// and a spot in its letter. Sizes are in subpixels (term::Canvas). `scale` is the letters'
// pixel size; the binary's squares are twice that, so the bytes are a bit wider than the
// letters, as on the site.

use crate::term::{Canvas, Rgb};

// 5×7 pixel letters; rows past the 7th are descenders. The same as PixelWord.astro's.
// Unknown characters get a solid block.
const GLYPHS: &[(char, &[&str])] = &[
    ('b', &["X....", "X....", "XXXX.", "X...X", "X...X", "X...X", "XXXX."]),
    ('g', &[".....", ".....", ".XXXX", "X...X", "X...X", "X...X", ".XXXX", "....X", "XXXX."]),
    ('h', &["X....", "X....", "XXXX.", "X...X", "X...X", "X...X", "X...X"]),
    ('i', &["..X..", ".....", ".XX..", "..X..", "..X..", "..X..", ".XXX."]),
    ('k', &["X....", "X....", "X..X.", "X.X..", "XX...", "X.X..", "X..X."]),
    ('l', &[".XX..", "..X..", "..X..", "..X..", "..X..", "..X..", ".XXX."]),
    ('n', &[".....", ".....", "XXXX.", "X...X", "X...X", "X...X", "X...X"]),
    ('s', &[".....", ".....", ".XXXX", "X....", ".XXX.", "....X", "XXXX."]),
    ('t', &[".X...", ".X...", "XXXX.", ".X...", ".X...", ".X..X", "..XX."]),
    ('0', &[".XXX.", "X...X", "X..XX", "X.X.X", "XX..X", "X...X", ".XXX."]),
    ('4', &["...X.", "..XX.", ".X.X.", "X..X.", "XXXXX", "...X.", "...X."]),
];
const BLOCK: &[&str] = &["XXXXX"; 7];
const ROWS: usize = 3; // bytes per column

const MINT: Rgb = crate::app::MINT;
// an unset bit's outline: rgba(154, 152, 232, .45) over the background
const EDGE: Rgb = Rgb(76, 74, 120);

struct Sq {
    // in the binary, and in the letter
    bin: (i32, i32),
    letter: (i32, i32),
    /// how long after a change it starts to move, in seconds
    d: f32,
    on: bool,
    /// letters with more than 8 dots get extra squares that fade in
    extra: bool,
}

pub struct Word {
    /// binary's width, the letters' width, and the height of whichever is taller
    pub w0: i32,
    pub w1: i32,
    pub h: i32,
    s: i32,
    p: i32,
    squares: Vec<Sq>,
    last: f32,
}

impl Word {
    pub fn new(word: &str, scale: i32) -> Word {
        let (s, g, p) = (2 * scale, 1, scale);
        let chars: Vec<char> = word.chars().collect();
        let glyph = |c: char| GLYPHS.iter().find(|g| g.0 == c).map_or(BLOCK, |g| g.1);
        let cols = chars.len().div_ceil(ROWS) as i32;
        let byte_w = 8 * s + 7 * g;
        let w0 = cols * byte_w + (cols - 1) * (s + g);
        let w1 = chars.len() as i32 * 6 * p - p;
        let glyph_rows = chars.iter().map(|&c| glyph(c).len()).max().unwrap_or(7) as i32;
        let h = (glyph_rows * p).max(chars.len().min(ROWS) as i32 * (s + g) - g);

        let mut squares = Vec::new();
        for (i, &c) in chars.iter().enumerate() {
            let byte = format!("{:08b}", (c as u32).min(255));
            let dots: Vec<(i32, i32)> = glyph(c)
                .iter()
                .enumerate()
                .flat_map(|(r, row)| {
                    row.chars().enumerate().filter(|&(_, ch)| ch == 'X').map(move |(col, _)| (col as i32, r as i32))
                })
                .collect();
            let (col, row) = ((i / ROWS) as i32, (i % ROWS) as i32);
            for k in 0..dots.len().max(8) {
                let (dx, dy) = dots[k.min(dots.len() - 1)];
                let j = (k % 8) as i32;
                // a column of bytes is 8 squares and 7 gaps wide, plus a square and a gap between columns
                let (a, b) = (9 * col + j, 8 * col + j);
                squares.push(Sq {
                    bin: (a * s + b * g, row * (s + g)),
                    letter: (i as i32 * 6 * p + dx * p, dy * p),
                    d: (i as f32 * 110.0 + k as f32 * 9.0) / 1000.0,
                    on: k < 8 && byte.as_bytes()[j as usize] == b'1',
                    extra: k >= 8,
                });
            }
        }
        let last = squares.iter().map(|q| q.d).fold(0.0, f32::max);
        Word { w0, w1, h, s, p, squares, last }
    }

    pub fn rows(&self) -> i32 {
        (self.h + 1) / 2
    }

    /// Draws the word at (x, y) in subpixels. `decoded`: letters or binary; `since`: seconds
    /// since it last changed (anything large once it's settled).
    pub fn draw(&self, c: &mut Canvas, x: i32, y: i32, decoded: bool, since: f32, bg: Rgb) {
        let clamp01 = |v: f32| v.clamp(0.0, 1.0);
        // extras first, so they sit under the squares they grow out of
        for q in self.squares.iter().filter(|q| q.extra).chain(self.squares.iter().filter(|q| !q.extra)) {
            // decoding goes letter by letter; going back runs the other way round
            let delay = if decoded { q.d } else { self.last - q.d };
            let m = ease_move(clamp01((since - delay) / 0.6));
            let ((fx, fy, fs), (tx, ty, ts)) = if decoded {
                ((q.bin.0, q.bin.1, self.s), (q.letter.0, q.letter.1, self.p))
            } else {
                ((q.letter.0, q.letter.1, self.p), (q.bin.0, q.bin.1, self.s))
            };
            let lerp = |a: i32, b: i32| (a as f32 + (b - a) as f32 * m).round() as i32;
            let (px, py, size) = (x + lerp(fx, tx), y + lerp(fy, ty), lerp(fs, ts).max(1));

            // how far toward mint: unset bits fill in a little after they start moving
            let fill = if decoded { clamp01((since - q.d - 0.1) / 0.3) } else { 1.0 - clamp01((since - delay) / 0.3) };
            if q.extra {
                let a = if decoded { clamp01((since - q.d) / 0.3) } else { 1.0 - clamp01((since - delay) / 0.3) };
                if a > 0.02 {
                    c.fill(px, py, size, size, bg.lerp(MINT, a));
                }
            } else if q.on {
                c.fill(px, py, size, size, MINT);
            } else {
                let edge = EDGE.lerp(MINT, fill);
                if size >= 3 {
                    // an outline, and the inside fills as it turns mint
                    c.fill(px, py, size, 1, edge);
                    c.fill(px, py + size - 1, size, 1, edge);
                    c.fill(px, py + 1, 1, size - 2, edge);
                    c.fill(px + size - 1, py + 1, 1, size - 2, edge);
                    if fill > 0.02 {
                        c.fill(px + 1, py + 1, size - 2, size - 2, bg.lerp(MINT, fill));
                    }
                } else {
                    c.fill(px, py, size, size, edge);
                }
            }
        }
    }
}

/// The squares' move: cubic-bezier(.5, 0, .2, 1), as on the site.
fn ease_move(x: f32) -> f32 {
    bezier(0.5, 0.0, 0.2, 1.0, x)
}

/// CSS's cubic-bezier timing function.
pub fn bezier(x1: f32, y1: f32, x2: f32, y2: f32, x: f32) -> f32 {
    if x <= 0.0 {
        return 0.0;
    }
    if x >= 1.0 {
        return 1.0;
    }
    let curve = |a: f32, b: f32, t: f32| 3.0 * a * t * (1.0 - t).powi(2) + 3.0 * b * t * t * (1.0 - t) + t.powi(3);
    // find t where the curve's x is x, by bisection; x(t) only rises
    let (mut lo, mut hi) = (0.0f32, 1.0f32);
    for _ in 0..24 {
        let mid = (lo + hi) / 2.0;
        if curve(x1, x2, mid) < x { lo = mid } else { hi = mid }
    }
    curve(y1, y2, (lo + hi) / 2.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes() {
        // "bit" at scale 2: 39 wide as bytes, 34 as letters, 14 tall either way
        let w = Word::new("bit", 2);
        assert_eq!((w.w0, w.w1, w.h), (39, 34, 14));
        // "things": the g's descender makes it 9 tall
        let w = Word::new("things", 1);
        assert_eq!((w.w0, w.w1, w.h), (49, 35, 9));
    }

    #[test]
    fn easing_ends() {
        assert_eq!(bezier(0.5, 0.0, 0.2, 1.0, 0.0), 0.0);
        assert_eq!(bezier(0.5, 0.0, 0.2, 1.0, 1.0), 1.0);
        let mid = bezier(0.5, 0.0, 0.2, 1.0, 0.5);
        assert!(mid > 0.5 && mid < 0.9, "{mid}");
    }
}
