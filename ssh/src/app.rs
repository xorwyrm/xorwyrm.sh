// The site in a terminal: the same bar on top, one column of content on the left and the
// planet on the right (or a horizon along the bottom when the window is narrow), the same
// pages, colours and pixel words. Built fresh every frame from the app's state.

use std::collections::HashMap;

use crate::input::{Event, Key, Mouse};
use crate::planet::{Planet, View};
use crate::site::site;
use crate::term::{self, Buffer, Canvas, Dots, Rect, Rgb, Style};
use crate::word::{Word, bezier};

// the site's colours (global.css)
pub const BG: Rgb = Rgb::hex(0x0d0b1c);
pub const INK: Rgb = Rgb::hex(0xecebf7);
pub const BODY: Rgb = Rgb::hex(0xa9a5c8);
pub const SOFT: Rgb = Rgb::hex(0x9b97bc);
pub const PRE: Rgb = Rgb::hex(0x8f8bb0);
pub const DIM: Rgb = Rgb::hex(0x6d6993);
pub const FAINT: Rgb = Rgb::hex(0x4f4c74);
pub const MINT: Rgb = Rgb::hex(0x7fd8b4);

/// The text column is at most this wide.
const COL_MAX: i32 = 56;
/// On a first visit the name decodes itself this long after the page opens.
const DECODE_AFTER: f32 = 1.0;
/// The pointer counts as gone after this long without moving: terminals don't say when it
/// leaves the window.
const POINTER_IDLE: f32 = 6.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Page {
    About,
    Things,
    Links,
    NotFound,
}

const NAV: [(Page, &str); 3] = [(Page::About, "about"), (Page::Things, "things"), (Page::Links, "links")];

impl Page {
    /// A page from a path, as in `ssh -t term.xorwyrm.sh things`.
    pub fn from_path(p: &str) -> Page {
        match p.trim().trim_matches('/').to_ascii_lowercase().as_str() {
            "" | "about" | "index" | "home" => Page::About,
            "things" => Page::Things,
            "links" => Page::Links,
            _ => Page::NotFound,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Target {
    Home,
    Nav(Page),
    Go(Page),
    Url(&'static str),
    Word,
}

struct Span {
    text: String,
    fg: Rgb,
    /// its colour under the pointer, or when it has focus
    hover: Rgb,
    bold: bool,
    ul: bool,
    target: Option<Target>,
}

impl Span {
    fn plain(text: impl Into<String>, fg: Rgb) -> Span {
        Span { text: text.into(), fg, hover: fg, bold: false, ul: false, target: None }
    }
}

enum Line {
    Blank,
    Text(Vec<Span>),
    /// a pixel word, at a scale
    Word(String, i32),
}

/// Where everything goes, for a window size.
#[derive(Clone, Copy, Debug)]
struct Layout {
    pad: i32,
    header_y: i32,
    col_x: i32,
    col_w: i32,
    /// rows the content can use: top, and one past the bottom
    top: i32,
    bottom: i32,
    hero: i32,
    view: View,
}

struct Swap {
    from: Page,
    at: f32,
}

pub struct App {
    w: i32,
    h: i32,
    page: Page,
    swap: Option<Swap>,
    time: f32,
    planet: Planet,
    layout: Layout,
    words: HashMap<(String, i32), Word>,

    // the name: letters or binary, when it last changed, and whether it was remembered
    decoded: bool,
    name_at: f32,
    name_known: bool,
    name_changed: Option<bool>,

    pointer: Option<(i32, i32)>,
    pointer_at: f32,
    hover: Option<Target>,
    focus: Option<usize>,
    scroll: i32,
    toast: Option<(String, f32)>,
    /// what's under each part of the last frame
    hits: Vec<(Rect, Target)>,
    /// the content's box in the last frame
    content: Rect,
    out: Vec<u8>,
    pub quit: bool,
}

impl App {
    /// `name`: whether this visitor last left the name decoded, if we know them.
    pub fn new(w: i32, h: i32, page: Page, name: Option<bool>) -> App {
        let mut app = App {
            w: 0,
            h: 0,
            page,
            swap: None,
            time: 0.0,
            planet: Planet::new(),
            layout: Layout {
                pad: 0,
                header_y: 0,
                col_x: 0,
                col_w: 0,
                top: 0,
                bottom: 0,
                hero: 2,
                view: View { cx: 0.0, cy: 0.0, r: 0.0, front: [0.0, 0.0, 1.0] },
            },
            words: HashMap::new(),
            decoded: name.unwrap_or(false),
            name_at: -100.0,
            name_known: name.is_some(),
            name_changed: None,
            pointer: None,
            pointer_at: -100.0,
            hover: None,
            focus: None,
            scroll: 0,
            toast: None,
            hits: Vec::new(),
            content: Rect { x: 0, y: 0, w: 0, h: 0 },
            out: Vec::new(),
            quit: false,
        };
        app.resize(w, h);
        app
    }

    pub fn resize(&mut self, w: i32, h: i32) {
        self.w = w.max(1);
        self.h = h.max(1);
        self.layout = self.compute_layout();
        self.planet.set_view(self.layout.view);
        self.scroll = self.scroll.clamp(0, self.max_scroll());
    }

    /// Bytes to send that aren't the picture (copying to the clipboard).
    pub fn take_output(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.out)
    }

    /// The name's new state, when it changes, to remember for next time.
    pub fn take_name(&mut self) -> Option<bool> {
        self.name_changed.take()
    }

    fn compute_layout(&self) -> Layout {
        let (w, h) = (self.w, self.h);
        let pad = ((w as f32 * 0.084).round() as i32).clamp(2, 16);
        let header_y = if h >= 16 { 1 } else { 0 };
        let top = header_y + 2;
        // the bottom row is for the key hints
        let bottom = (h - 1).max(top);
        let col_w = COL_MAX.min(w - 2 * pad).max(1);
        let hero = if col_w >= 40 { 2 } else { 1 };
        let mut l = Layout {
            pad,
            header_y,
            col_x: pad,
            col_w,
            top,
            bottom,
            hero,
            view: View { cx: 0.0, cy: 0.0, r: 0.0, front: [0.0, 0.0, 1.0] },
        };

        // in subpixels: a cell is one across and two down
        let (sw, sh) = (w as f32, h as f32 * 2.0);
        // wide: fully round on the right, clear of the text. on smaller windows the column and
        // the margin give way first (down to 40 and 3), so 80×24 still gets the planet
        let wide = |pad: i32, col_w: i32| {
            let mut r = (0.36 * sh).min(0.21 * sw);
            let mut cx = (0.745 * sw).min(0.95 * sw - r);
            let left = (pad + col_w + 3) as f32;
            if cx - r < left {
                r = r.min((0.97 * sw - left) / 2.0);
                cx = left + r;
            }
            (r >= 12.0).then_some((cx, r))
        };
        let tries = (40..=col_w).rev().map(|c| (pad, c)).chain((40..=COL_MAX.min(w - 6)).rev().map(|c| (3, c)));
        for (p, c) in tries {
            if let Some((cx, r)) = wide(p, c) {
                l.pad = p;
                l.col_x = p;
                l.col_w = c;
                l.hero = if c >= 40 { 2 } else { 1 };
                l.view = View { cx, cy: sh / 2.0, r, front: [-0.2, 0.25, 1.0] };
                return l;
            }
        }

        // narrow: a horizon along the bottom, below the tallest page
        let tallest = [Page::About, Page::Things, Page::Links, Page::NotFound]
            .iter()
            .map(|&p| self.height_of(&self.lines(p, &l)))
            .max()
            .unwrap_or(0);
        let r = 0.62 * sw;
        let horizon = (((top + tallest + 1) * 2) as f32).max(sh - 0.55 * r);
        l.view = View { cx: sw / 2.0, cy: horizon + r, r, front: [0.0, 0.8, 0.6] };
        // the content sits between the bar and the horizon
        l.bottom = l.bottom.min((horizon / 2.0).floor() as i32).max(top);
        l
    }

    fn word(&mut self, text: &str, scale: i32) -> &Word {
        self.words.entry((text.to_string(), scale)).or_insert_with(|| Word::new(text, scale))
    }

    fn word_rows(&self, text: &str, scale: i32) -> i32 {
        self.words.get(&(text.to_string(), scale)).map_or_else(|| Word::new(text, scale).rows(), |w| w.rows())
    }

    fn height_of(&self, lines: &[Line]) -> i32 {
        lines
            .iter()
            .map(|l| match l {
                Line::Blank | Line::Text(_) => 1,
                Line::Word(t, s) => self.word_rows(t, *s),
            })
            .sum()
    }

    /// A page's content, laid out for the column.
    fn lines(&self, page: Page, l: &Layout) -> Vec<Line> {
        let s = site();
        let width = l.col_w;
        let mut out = Vec::new();
        let title = |out: &mut Vec<Line>, t: &str| {
            out.push(Line::Word(t.to_string(), 1));
            out.push(Line::Blank);
        };
        match page {
            Page::About => {
                out.push(Line::Text(vec![Span::plain("hi, i'm", PRE)]));
                out.push(Line::Blank);
                out.push(Line::Word(s.site.name.clone(), l.hero));
                out.push(Line::Blank);
                // *stars* mark the words that stand out
                let parts: Vec<(&str, bool)> = s.intro.split('*').enumerate().map(|(i, p)| (p, i % 2 == 1)).collect();
                for line in wrap(&parts, width) {
                    out.push(Line::Text(
                        line.into_iter()
                            .map(|(t, em)| Span { bold: em, ..Span::plain(t, if em { INK } else { BODY }) })
                            .collect(),
                    ));
                }
                out.push(Line::Blank);
                out.push(Line::Text(vec![Span {
                    hover: MINT,
                    target: Some(Target::Go(Page::Things)),
                    ..Span::plain("things i've made →", DIM)
                }]));
            }
            Page::Things => {
                title(&mut out, "things");
                for (i, p) in s.projects.iter().enumerate() {
                    if i > 0 {
                        out.push(Line::Blank);
                    }
                    // the whole project is the link; its name lights up
                    let target = Some(Target::Url(&p.url));
                    out.push(Line::Text(vec![Span {
                        text: p.name.clone(),
                        fg: INK,
                        hover: MINT,
                        bold: true,
                        ul: true,
                        target,
                    }]));
                    for line in wrap(&[(p.desc.as_str(), false)], width) {
                        let text: String = line.into_iter().map(|(t, _)| t).collect();
                        out.push(Line::Text(vec![Span { target, ..Span::plain(text, SOFT) }]));
                    }
                }
            }
            Page::Links => {
                title(&mut out, "links");
                let label_w = s.links.iter().map(|k| term::width(&k.label)).max().unwrap_or(0) + 3;
                for k in &s.links {
                    let pad = " ".repeat((label_w - term::width(&k.label)).max(1) as usize);
                    out.push(Line::Text(vec![
                        Span::plain(format!("{}{pad}", k.label), DIM),
                        Span { hover: MINT, ul: true, target: Some(Target::Url(&k.url)), ..Span::plain(&k.text, INK) },
                    ]));
                }
            }
            Page::NotFound => {
                title(&mut out, "404");
                out.push(Line::Text(vec![Span::plain("nothing lives here.", BODY)]));
                out.push(Line::Blank);
                out.push(Line::Text(vec![Span {
                    hover: MINT,
                    target: Some(Target::Go(Page::About)),
                    ..Span::plain("go home →", DIM)
                }]));
            }
        }
        out
    }

    /// The page's links, in order, for tab; with the row each is on.
    fn focusables(&self, page: Page) -> Vec<(Target, i32)> {
        let mut out: Vec<(Target, i32)> = Vec::new();
        let mut y = 0;
        for line in self.lines(page, &self.layout) {
            match &line {
                Line::Text(spans) => {
                    for t in spans.iter().filter_map(|s| s.target) {
                        if !out.iter().any(|(o, _)| *o == t) {
                            out.push((t, y));
                        }
                    }
                    y += 1;
                }
                Line::Blank => y += 1,
                Line::Word(t, s) => y += self.word_rows(t, *s),
            }
        }
        out
    }

    fn avail(&self) -> i32 {
        self.layout.bottom - self.layout.top
    }

    fn max_scroll(&self) -> i32 {
        (self.height_of(&self.lines(self.page, &self.layout)) - self.avail()).max(0)
    }

    fn go(&mut self, page: Page) {
        if page == self.page && self.swap.is_none() {
            return;
        }
        self.swap = Some(Swap { from: self.page, at: self.time });
        self.page = page;
        self.focus = None;
        self.hover = None;
        self.scroll = 0;
    }

    fn set_name(&mut self, decoded: bool) {
        self.decoded = decoded;
        self.name_at = self.time;
        self.name_known = true;
        self.name_changed = Some(decoded);
    }

    fn activate(&mut self, t: Target) {
        match t {
            Target::Home => self.go(Page::About),
            Target::Nav(p) | Target::Go(p) => self.go(p),
            Target::Word => self.set_name(!self.decoded),
            Target::Url(url) => {
                // the terminal can't open a browser for us; hand the link over instead, through
                // the clipboard (OSC 52). the link text is a terminal hyperlink too (OSC 8)
                self.out.extend_from_slice(format!("\x1b]52;c;{}\x07", base64(url.as_bytes())).as_bytes());
                self.toast = Some((format!("copied {url}"), self.time));
            }
        }
    }

    fn move_focus(&mut self, by: i32) {
        let f = self.focusables(self.page);
        if f.is_empty() {
            return;
        }
        let n = f.len() as i32;
        let i = match self.focus {
            None if by > 0 => 0,
            None => n - 1,
            Some(i) => (i as i32 + by).rem_euclid(n),
        };
        self.focus = Some(i as usize);
        // keep it in view
        let y = f[i as usize].1;
        let avail = self.avail();
        if y < self.scroll {
            self.scroll = y;
        } else if y >= self.scroll + avail {
            self.scroll = y - avail + 1;
        }
        self.scroll = self.scroll.clamp(0, self.max_scroll());
    }

    fn nav_step(&mut self, by: i32) {
        let i = NAV.iter().position(|(p, _)| *p == self.page).map_or(0, |i| (i as i32 + by).rem_euclid(3));
        self.go(NAV[i as usize].0);
    }

    pub fn input(&mut self, ev: Event) {
        match ev {
            Event::Key(k) => match k {
                Key::Quit | Key::Char('q') | Key::Char('Q') => self.quit = true,
                Key::Esc => {
                    if self.focus.is_some() {
                        self.focus = None;
                    } else {
                        self.quit = true;
                    }
                }
                // number keys switch pages, as on the site
                Key::Char(c @ '1'..='3') => self.go(NAV[(c as u8 - b'1') as usize].0),
                Key::Left | Key::Char('h') => self.nav_step(-1),
                Key::Right | Key::Char('l') => self.nav_step(1),
                Key::Tab | Key::Down | Key::Char('j') => self.move_focus(1),
                Key::BackTab | Key::Up | Key::Char('k') => self.move_focus(-1),
                Key::Enter | Key::Char(' ') => {
                    let f = self.focusables(self.page);
                    if let Some(&(t, _)) = self.focus.and_then(|i| f.get(i)) {
                        self.activate(t);
                    }
                }
                Key::PageUp => self.scroll = (self.scroll - self.avail() / 2).max(0),
                Key::PageDown => self.scroll = (self.scroll + self.avail() / 2).min(self.max_scroll()),
                Key::Home => self.scroll = 0,
                Key::End => self.scroll = self.max_scroll(),
                _ => {}
            },
            Event::Mouse(kind, x, y) => {
                self.pointer = Some((x, y));
                self.pointer_at = self.time;
                self.hover = self.hit(x, y);
                match kind {
                    Mouse::Press => {
                        if let Some(t) = self.hover {
                            self.focus = None;
                            self.activate(t);
                        } else if !self.over_ui(x, y) {
                            self.planet.click(x as f32 + 0.5, y as f32 * 2.0 + 1.0);
                        }
                    }
                    Mouse::ScrollUp => self.scroll = (self.scroll - 2).max(0),
                    Mouse::ScrollDown => self.scroll = (self.scroll + 2).min(self.max_scroll()),
                    Mouse::Move | Mouse::Release => {}
                }
                self.update_pointer();
            }
        }
    }

    fn hit(&self, x: i32, y: i32) -> Option<Target> {
        self.hits.iter().find(|(r, _)| r.contains(x, y)).map(|(_, t)| *t)
    }

    /// Over the bar or the text, where the planet doesn't take the pointer.
    fn over_ui(&self, x: i32, y: i32) -> bool {
        y == self.layout.header_y || self.content.contains(x, y)
    }

    fn update_pointer(&mut self) {
        let ndc = |x: i32| (x as f32 + 0.5) / self.w as f32 * 2.0 - 1.0;
        match self.pointer {
            Some((x, y)) if self.time - self.pointer_at < POINTER_IDLE => {
                let over = (!self.over_ui(x, y) && self.hover.is_none()).then(|| (x as f32 + 0.5, y as f32 * 2.0 + 1.0));
                self.planet.set_pointer(over, ndc(x));
            }
            Some((x, _)) => self.planet.set_pointer(None, ndc(x)),
            None => self.planet.set_pointer(None, 0.0),
        }
    }

    pub fn tick(&mut self, dt: f32) {
        self.time += dt;
        // first visit: the name decodes on its own
        if !self.name_known && self.time >= DECODE_AFTER {
            self.set_name(true);
        }
        if self.swap.as_ref().is_some_and(|s| self.time - s.at > 0.4) {
            self.swap = None;
        }
        if self.toast.as_ref().is_some_and(|(_, at)| self.time - at > 3.0) {
            self.toast = None;
        }
        self.update_pointer();
        self.planet.update(dt);
    }

    pub fn draw(&mut self) -> Buffer {
        let mut buf = Buffer::new(self.w, self.h, BG);
        let full = Rect { x: 0, y: 0, w: self.w, h: self.h };
        self.hits.clear();

        let mut dots = Dots::new(self.w * 2, self.h * 4);
        self.planet.draw(&mut dots);
        dots.blit(&mut buf, full);

        self.draw_header(&mut buf);

        // page changes: the old content fades out, then the new content fades in, as on the site
        let (page, fade) = match &self.swap {
            Some(s) => {
                let t = self.time - s.at;
                if t < 0.1 {
                    (Some(s.from), 1.0 - bezier(0.42, 0.0, 1.0, 1.0, t / 0.1))
                } else if t < 0.11 {
                    (None, 0.0)
                } else {
                    (Some(self.page), bezier(0.2, 0.7, 0.2, 1.0, ((t - 0.11) / 0.26).min(1.0)))
                }
            }
            None => (Some(self.page), 1.0),
        };
        if let Some(p) = page {
            let scroll = if p == self.page { self.scroll } else { 0 };
            self.draw_content(&mut buf, p, fade, scroll);
        }

        self.draw_footer(&mut buf);
        buf
    }

    fn draw_header(&mut self, buf: &mut Buffer) {
        let l = self.layout;
        let y = l.header_y;
        let full = Rect { x: 0, y: 0, w: self.w, h: self.h };
        let s = site();
        let home = &s.site.domain;
        let hov = self.hover == Some(Target::Home);
        let end = buf.text(l.pad, y, home, Style { bold: true, ..Style::fg(if hov { MINT } else { INK }) }, full);
        self.hits.push((Rect { x: l.pad, y, w: end - l.pad, h: 1 }, Target::Home));

        let total: i32 = NAV.iter().map(|(_, n)| term::width(n)).sum::<i32>() + 3 * (NAV.len() as i32 - 1);
        let mut x = (self.w - l.pad - total).max(end + 2);
        for (p, name) in NAV {
            let t = Target::Nav(p);
            let fg = if p == self.page {
                MINT
            } else if self.hover == Some(t) {
                INK
            } else {
                DIM
            };
            let e = buf.text(x, y, name, Style::fg(fg), full);
            self.hits.push((Rect { x, y, w: e - x, h: 1 }, t));
            x = e + 3;
        }
    }

    fn draw_content(&mut self, buf: &mut Buffer, page: Page, fade: f32, scroll: i32) {
        let l = self.layout;
        let lines = self.lines(page, &l);
        let height = self.height_of(&lines);
        let avail = l.bottom - l.top;
        // centred in the space it has, or scrolled when it doesn't fit
        let top = if height <= avail { l.top + (avail - height) / 2 } else { l.top - scroll };
        let clip = Rect { x: l.col_x, y: l.top, w: l.col_w, h: avail };
        self.content = Rect { x: l.col_x, y: top.max(l.top), w: l.col_w, h: height.min(avail) };
        let f = |c: Rgb| if fade >= 1.0 { c } else { BG.lerp(c, fade) };
        let focused = self.focus.and_then(|i| self.focusables(page).get(i).map(|f| f.0));

        let mut y = top;
        for line in &lines {
            match line {
                Line::Blank => y += 1,
                Line::Text(spans) => {
                    let mut x = l.col_x;
                    for sp in spans {
                        let active = sp.target.is_some() && (sp.target == self.hover || sp.target == focused);
                        let link = match sp.target {
                            Some(Target::Url(u)) => buf.link(u),
                            _ => 0,
                        };
                        let st = Style {
                            fg: f(if active { sp.hover } else { sp.fg }),
                            bold: sp.bold,
                            ul: sp.ul || (active && sp.target == focused && sp.hover != sp.fg),
                            link,
                        };
                        let end = buf.text(x, y, &sp.text, st, clip);
                        if let Some(t) = sp.target
                            && clip.contains(x, y)
                            && fade > 0.5
                        {
                            self.hits.push((Rect { x, y, w: end - x, h: 1 }, t));
                        }
                        x = end;
                    }
                    y += 1;
                }
                Line::Word(text, scale) => {
                    let (decoded, since) = (self.decoded, self.time - self.name_at);
                    let w = self.word(text, *scale);
                    let mut c = Canvas::new(w.w0.max(w.w1), w.h);
                    w.draw(&mut c, 0, 0, decoded, since, BG);
                    let (rows, width) = (w.rows(), if decoded { w.w1 } else { w.w0 });
                    c.blit(buf, l.col_x, y, clip, fade);
                    if fade > 0.5 {
                        self.hits.push((Rect { x: l.col_x, y, w: width, h: rows }, Target::Word));
                    }
                    y += rows;
                }
            }
        }
    }

    fn draw_footer(&mut self, buf: &mut Buffer) {
        let y = self.h - 1;
        if y <= self.layout.header_y {
            return;
        }
        let full = Rect { x: 0, y: 0, w: self.w, h: self.h };
        let (text, fg) = match &self.toast {
            Some((t, _)) => (t.clone(), MINT),
            None => {
                let room = self.w - 2 * self.layout.pad;
                let hint = [
                    "←/→ to switch pages · tab to pick a link · enter to copy it · q to quit",
                    "←/→ to switch pages · tab to pick a link · q to quit",
                    "←/→ to switch pages · q to quit",
                    "q to quit",
                ]
                    .into_iter()
                    .find(|h| term::width(h) <= room)
                    .unwrap_or("");
                (hint.to_string(), FAINT)
            }
        };
        let w = term::width(&text);
        if w > 0 {
            // a clean strip, so it reads over the horizon
            buf.clear(self.layout.pad - 1, y, w + 2);
            buf.text(self.layout.pad, y, &text, Style::fg(fg), full);
        }
    }
}

/// Breaks styled text into lines at most `width` wide, at spaces. A word can change style
/// part way ("research*." is a word in two styles).
fn wrap<S: Copy + PartialEq>(parts: &[(&str, S)], width: i32) -> Vec<Vec<(String, S)>> {
    // words, each a list of styled pieces. a word only ends at a space, not where the style
    // changes, so "research" and the "." after it stay together
    let mut words: Vec<Vec<(String, S)>> = Vec::new();
    let mut cur: Vec<(String, S)> = Vec::new();
    for &(text, st) in parts {
        for ch in text.chars() {
            if ch.is_whitespace() {
                if !cur.is_empty() {
                    words.push(std::mem::take(&mut cur));
                }
            } else {
                push_piece(&mut cur, ch, st);
            }
        }
    }
    if !cur.is_empty() {
        words.push(cur);
    }
    let mut lines: Vec<Vec<(String, S)>> = Vec::new();
    let mut line: Vec<(String, S)> = Vec::new();
    let mut lw = 0;
    for pieces in words {
        let ww: i32 = pieces.iter().map(|(t, _)| term::width(t)).sum();
        if lw > 0 && lw + 1 + ww > width {
            lines.push(std::mem::take(&mut line));
            lw = 0;
        }
        if lw > 0 {
            // the space takes the style of the word before it
            let st = line.last().unwrap().1;
            line.push((" ".into(), st));
            lw += 1;
        }
        lw += ww;
        line.extend(pieces);
    }
    if !line.is_empty() {
        lines.push(line);
    }
    // merge neighbouring pieces in the same style
    lines
        .into_iter()
        .map(|l| {
            let mut out: Vec<(String, S)> = Vec::new();
            for (t, s) in l {
                match out.last_mut() {
                    Some((pt, ps)) if *ps == s => pt.push_str(&t),
                    _ => out.push((t, s)),
                }
            }
            out
        })
        .collect()
}

fn push_piece<S: Copy + PartialEq>(pieces: &mut Vec<(String, S)>, ch: char, st: S) {
    match pieces.last_mut() {
        Some((t, s)) if *s == st => t.push(ch),
        _ => pieces.push((ch.to_string(), st)),
    }
}

fn base64(data: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::new();
    for c in data.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            if i <= c.len() {
                s.push(A[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                s.push('=');
            }
        }
    }
    s
}

/// The site as plain text, for `ssh term.xorwyrm.sh` without a terminal (piped, or `ssh -T`).
pub fn plain(page: Page) -> String {
    let s = site();
    let mut o = String::new();
    let wrapped = |o: &mut String, text: &str, indent: &str| {
        for line in wrap(&[(text, ())], 72 - indent.len() as i32) {
            o.push_str(indent);
            o.extend(line.into_iter().map(|(t, _)| t));
            o.push('\n');
        }
    };
    match page {
        Page::About => {
            o.push_str(&format!("hi, i'm {}\n\n", s.site.name));
            wrapped(&mut o, &s.intro.replace('*', ""), "");
            o.push_str("\nthings i've made: ssh term.xorwyrm.sh things\n");
        }
        Page::Things => {
            o.push_str("things\n");
            for p in &s.projects {
                o.push_str(&format!("\n{}  {}\n", p.name, p.url));
                wrapped(&mut o, &p.desc, "  ");
            }
        }
        Page::Links => {
            o.push_str("links\n\n");
            let w = s.links.iter().map(|k| k.label.len()).max().unwrap_or(0);
            for k in &s.links {
                o.push_str(&format!("{:w$}   {}\n", k.label, k.url));
            }
        }
        Page::NotFound => o.push_str("404: nothing lives here. try: about, things, links\n"),
    }
    o
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wraps_with_glued_words() {
        let parts = [("deep in ", false), ("security research", true), (". nixos", false)];
        let lines = wrap(&parts, 20);
        let flat: Vec<String> = lines.iter().map(|l| l.iter().map(|(t, _)| t.as_str()).collect()).collect();
        assert_eq!(flat, vec!["deep in security", "research. nixos"]);
        // "research" stays emphasised and the full stop after it doesn't
        assert_eq!(lines[1][0], ("research".to_string(), true));
        assert_eq!(lines[1][1], (". nixos".to_string(), false));
    }

    fn rng(mut seed: u64) -> impl FnMut(u64) -> u64 {
        move |below| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed % below.max(1)
        }
    }

    /// Whatever a client sends, at whatever window size, nothing panics: random bytes, real
    /// and broken escape sequences, clicks on and off the screen, resizes part way through.
    /// (Run without optimisation, this also catches arithmetic that overflows.)
    #[test]
    fn survives_whatever_a_client_sends() {
        use crate::input::Parser;
        use crate::term::Screen;
        const PIECES: [&[u8]; 40] = [
            b"\x1b[<0;", b"\x1b[<64;", b"\x1b[<65;", b"\x1b[<35;", b"\x1b[<", b";", b"M", b"m", b"\x1b[", b"\x1bO",
            b"\x1b", b"\x1b\x1b", b"\x1b[M", b"A", b"B", b"C", b"D", b"~", b"5", b"6", b"\t", b"\r", b"j", b"k", b"h",
            b"l", b"1", b"2", b"3", b" ", b"Z", b"H", b"F", b"-", b"99999", b"2147483647", b"-2147483648", b"\xff",
            b"\xe2\x96", b"\xf0\x9f\x98\x80",
        ];
        let mut r = rng(0x2545_f491_4f6c_dd1d);
        // the sizes main.rs lets through: 10 to 500 across, 5 to 200 down
        let size = |r: &mut dyn FnMut(u64) -> u64| match r(10) {
            0 => (10, 5),
            1 => (10 + r(491) as i32, 5 + r(196) as i32),
            2 => (10 + r(30) as i32, 5 + r(196) as i32),
            3 => (10 + r(491) as i32, 5 + r(12) as i32),
            _ => (10 + r(150) as i32, 5 + r(50) as i32),
        };
        for round in 0..160 {
            let (w, h) = size(&mut r);
            let page = [Page::About, Page::Things, Page::Links, Page::NotFound][r(4) as usize];
            let mut app = App::new(w, h, page, [None, Some(false), Some(true)][r(3) as usize]);
            let (mut w, mut h) = (w, h);
            let mut parser = Parser::default();
            let mut screen = Screen::new();
            let mut out = Vec::new();
            for _ in 0..24 {
                let mut bytes: Vec<u8> = Vec::new();
                for _ in 0..r(12) {
                    match r(4) {
                        // a well-formed mouse report, on or near the screen
                        0 => {
                            let btn = [0, 0, 1, 2, 3, 32, 35, 64, 65][r(9) as usize];
                            let (x, y) = (r(w as u64 + 4) as i32 - 1, r(h as u64 + 4) as i32 - 1);
                            let fin = if r(4) == 0 { 'm' } else { 'M' };
                            bytes.extend_from_slice(format!("\x1b[<{btn};{x};{y}{fin}").as_bytes());
                        }
                        1 => bytes.extend((0..r(20)).map(|_| r(256) as u8)),
                        _ => bytes.extend_from_slice(PIECES[r(PIECES.len() as u64) as usize]),
                    }
                }
                for ev in parser.feed(&bytes) {
                    app.input(ev);
                }
                app.tick(r(101) as f32 / 1000.0);
                if r(6) == 0 {
                    (w, h) = size(&mut r);
                    app.resize(w, h);
                    screen.invalidate();
                }
                let frame = app.draw();
                assert_eq!((frame.w, frame.h), (w, h), "round {round}");
                out.clear();
                out.extend(app.take_output());
                screen.draw(&frame, &mut out);
                app.take_name();
            }
        }
    }

    /// Every width, at the heights where the layout changes, lays out and draws.
    #[test]
    fn every_width_lays_out() {
        for w in 10..=500 {
            for h in [5, 6, 7, 8, 12, 15, 16, 17, 24, 40, 200] {
                // the tall, wide ones are slow to draw and no different in kind: sample them
                if (w > 160 || h > 40) && w % 23 != 0 {
                    continue;
                }
                for page in [Page::About, Page::Things, Page::Links, Page::NotFound] {
                    let mut app = App::new(w, h, page, Some(true));
                    app.tick(0.04);
                    let frame = app.draw();
                    assert_eq!((frame.w, frame.h), (w, h));
                }
            }
        }
    }

    #[test]
    fn base64_matches() {
        assert_eq!(base64(b"hi"), "aGk=");
        assert_eq!(base64(b"abc"), "YWJj");
        assert_eq!(base64(b"abcd"), "YWJjZA==");
    }
}
