// Turns the bytes a terminal sends into keys and mouse events.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Char(char),
    Enter,
    Tab,
    BackTab,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    Esc,
    /// ctrl-c or ctrl-d
    Quit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mouse {
    Move,
    Press,
    Release,
    ScrollUp,
    ScrollDown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    Key(Key),
    /// x and y are cells, from 0
    Mouse(Mouse, i32, i32),
}

/// Holds on to a sequence cut off at the end of one read until the rest arrives.
#[derive(Default)]
pub struct Parser {
    pending: Vec<u8>,
}

impl Parser {
    pub fn feed(&mut self, data: &[u8]) -> Vec<Event> {
        self.pending.extend_from_slice(data);
        // nothing sensible is ever this long; drop junk rather than hold it
        if self.pending.len() > 4096 {
            self.pending.clear();
        }
        let buf = std::mem::take(&mut self.pending);
        let mut out = Vec::new();
        let mut i = 0;
        while i < buf.len() {
            match parse(&buf[i..]) {
                Parsed::Event(e, n) => {
                    out.extend(e);
                    i += n;
                }
                Parsed::Incomplete => {
                    // a lone ESC at the end is the Esc key: terminals send sequences whole
                    if buf.len() - i == 1 && buf[i] == 0x1b {
                        out.push(Event::Key(Key::Esc));
                        i += 1;
                    } else {
                        self.pending = buf[i..].to_vec();
                        break;
                    }
                }
            }
        }
        out
    }
}

enum Parsed {
    Event(Option<Event>, usize),
    Incomplete,
}

fn key(k: Key, n: usize) -> Parsed {
    Parsed::Event(Some(Event::Key(k)), n)
}

fn parse(b: &[u8]) -> Parsed {
    match b[0] {
        0x1b => escape(b),
        b'\r' | b'\n' => key(Key::Enter, 1),
        b'\t' => key(Key::Tab, 1),
        0x03 | 0x04 => key(Key::Quit, 1),
        c if c < 0x20 || c == 0x7f => Parsed::Event(None, 1),
        c => {
            // a UTF-8 character
            let n = match c {
                0x00..=0x7f => 1,
                0xc0..=0xdf => 2,
                0xe0..=0xef => 3,
                0xf0..=0xf7 => 4,
                _ => return Parsed::Event(None, 1),
            };
            if b.len() < n {
                return Parsed::Incomplete;
            }
            match std::str::from_utf8(&b[..n]).ok().and_then(|s| s.chars().next()) {
                Some(ch) => key(Key::Char(ch), n),
                None => Parsed::Event(None, 1),
            }
        }
    }
}

fn escape(b: &[u8]) -> Parsed {
    if b.len() < 2 {
        return Parsed::Incomplete;
    }
    match b[1] {
        b'[' => csi(b),
        b'O' => {
            // arrows in application cursor mode
            if b.len() < 3 {
                return Parsed::Incomplete;
            }
            let k = match b[2] {
                b'A' => Some(Key::Up),
                b'B' => Some(Key::Down),
                b'C' => Some(Key::Right),
                b'D' => Some(Key::Left),
                b'H' => Some(Key::Home),
                b'F' => Some(Key::End),
                _ => None,
            };
            Parsed::Event(k.map(Event::Key), 3)
        }
        0x1b => key(Key::Esc, 1),
        // alt and a key: ignored
        _ => Parsed::Event(None, 2),
    }
}

fn csi(b: &[u8]) -> Parsed {
    // old-style mouse report: ESC [ M and three bytes. we ask for SGR reports, so just skip it
    if b.get(2) == Some(&b'M') {
        return if b.len() < 6 { Parsed::Incomplete } else { Parsed::Event(None, 6) };
    }
    // parameters, then a final byte from @ to ~
    let Some(end) = b[2..].iter().position(|c| (0x40..=0x7e).contains(c)).map(|p| p + 2) else {
        return if b.len() > 32 { Parsed::Event(None, b.len()) } else { Parsed::Incomplete };
    };
    let params = &b[2..end];
    let n = end + 1;
    let fin = b[end];
    if params.first() == Some(&b'<') {
        return Parsed::Event(sgr_mouse(&params[1..], fin), n);
    }
    let k = match fin {
        b'A' => Some(Key::Up),
        b'B' => Some(Key::Down),
        b'C' => Some(Key::Right),
        b'D' => Some(Key::Left),
        b'H' => Some(Key::Home),
        b'F' => Some(Key::End),
        b'Z' => Some(Key::BackTab),
        b'~' => match params {
            b"1" | b"7" => Some(Key::Home),
            b"4" | b"8" => Some(Key::End),
            b"5" => Some(Key::PageUp),
            b"6" => Some(Key::PageDown),
            _ => None,
        },
        _ => None,
    };
    Parsed::Event(k.map(Event::Key), n)
}

/// ESC [ < button ; x ; y M (pressed or moved) or m (released)
fn sgr_mouse(params: &[u8], fin: u8) -> Option<Event> {
    let s = std::str::from_utf8(params).ok()?;
    let mut it = s.split(';').map(|p| p.parse::<i32>().ok());
    let (btn, x, y) = (it.next()??, it.next()??, it.next()??);
    // whatever numbers arrive, what comes out is a place on (or just off) a screen
    let (x, y) = (x.clamp(0, 10_000) - 1, y.clamp(0, 10_000) - 1);
    let kind = if btn & 64 != 0 {
        if btn & 1 == 0 { Mouse::ScrollUp } else { Mouse::ScrollDown }
    } else if btn & 32 != 0 {
        Mouse::Move
    } else if fin == b'm' {
        Mouse::Release
    } else if btn & 3 == 0 {
        // only the left button clicks
        Mouse::Press
    } else {
        return None;
    };
    Some(Event::Mouse(kind, x, y))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_and_mouse() {
        let mut p = Parser::default();
        let e = p.feed(b"q\x1b[A\x1b[<0;10;5M\x1b[<35;3;4M\x1b[<64;1;1M\r");
        assert_eq!(
            e,
            vec![
                Event::Key(Key::Char('q')),
                Event::Key(Key::Up),
                Event::Mouse(Mouse::Press, 9, 4),
                Event::Mouse(Mouse::Move, 2, 3),
                Event::Mouse(Mouse::ScrollUp, 0, 0),
                Event::Key(Key::Enter),
            ]
        );
    }

    #[test]
    fn split_sequence() {
        let mut p = Parser::default();
        assert!(p.feed(b"\x1b[<0;1").is_empty());
        assert_eq!(p.feed(b"0;5m"), vec![Event::Mouse(Mouse::Release, 9, 4)]);
        assert_eq!(p.feed(b"\x1b"), vec![Event::Key(Key::Esc)]);
    }

    #[test]
    fn absurd_numbers_are_tamed() {
        let mut p = Parser::default();
        assert_eq!(
            p.feed(b"\x1b[<0;-2147483648;2147483647M"),
            vec![Event::Mouse(Mouse::Press, -1, 9_999)]
        );
        // too big for a number at all: not an event
        assert!(p.feed(b"\x1b[<0;99999999999;1M").is_empty());
    }

    #[test]
    fn junk_is_not_kept() {
        let mut p = Parser::default();
        // an escape sequence that never ends
        for _ in 0..100 {
            p.feed(&[b'\x1b', b'[', b'1', b';', b'1', b';', b'1', b';']);
            p.feed(&[b'0'; 1000]);
        }
        assert!(p.pending.len() <= 4096);
    }
}
