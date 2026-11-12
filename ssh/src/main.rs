// xorwyrm.sh over ssh: `ssh xorwyrm.sh` opens the site in your terminal.
//
// An ssh server that runs no shell and no commands. Every session gets the site, drawn by
// app.rs. Anyone can log in: any key, or none (keyboard-interactive with no questions). The
// key, when there is one, is only used to remember whether you left the name as letters or
// binary, in memory, until the server restarts.
//
// `ssh -t xorwyrm.sh things` opens a page; without a terminal (`ssh xorwyrm.sh links`, or
// piped) it prints the page as plain text.
//
// Settings, from the environment:
//   XORWYRM_LISTEN    address to listen on (default 0.0.0.0:2222)
//   XORWYRM_HOST_KEY  the server's key, made on first run if missing (default ./host_key)

mod app;
mod input;
mod planet;
mod site;
mod term;
mod word;

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use russh::keys::ssh_key::LineEnding;
use russh::keys::{Algorithm, HashAlg, PrivateKey};
use russh::server::{self, Auth, Handle, Msg, Server as _, Session, VerifiedPublicKey};
use russh::{Channel, ChannelId, MethodKind, MethodSet, SshId};
use tokio::sync::mpsc;
use tokio::time::MissedTickBehavior;

use app::{App, Page};

/// Sessions at once; past this, newcomers are asked to come back later.
const MAX_SESSIONS: usize = 200;
/// A session ends after this long without a key press or mouse move.
const IDLE_LIMIT: Duration = Duration::from_secs(30 * 60);
/// About 25 frames a second.
const FRAME: Duration = Duration::from_millis(40);
/// How many remembered visitors to keep.
const MAX_NAMES: usize = 100_000;

struct Shared {
    /// by key fingerprint: whether the name was left decoded
    names: Mutex<HashMap<String, bool>>,
    sessions: AtomicUsize,
}

#[derive(Clone)]
struct Server {
    shared: Arc<Shared>,
}

impl server::Server for Server {
    type Handler = Conn;
    fn new_client(&mut self, _: Option<std::net::SocketAddr>) -> Conn {
        Conn { shared: self.shared.clone(), key: None, pty: None, input: None }
    }
    fn handle_session_error(&mut self, e: russh::Error) {
        eprintln!("session error: {e}");
    }
}

enum Input {
    Data(Vec<u8>),
    Resize(u32, u32),
    Close,
}

/// One ssh connection.
struct Conn {
    shared: Arc<Shared>,
    key: Option<String>,
    /// columns and rows, once a terminal is asked for
    pty: Option<(u32, u32)>,
    input: Option<mpsc::Sender<Input>>,
}

impl Conn {
    fn send(&self, i: Input) {
        if let Some(tx) = &self.input {
            // a full queue means the session is stuck or flooded: drop the input
            let _ = tx.try_send(i);
        }
    }

    fn start(&mut self, ch: ChannelId, command: &str, session: &mut Session) {
        let handle = session.handle();
        let page = Page::from_path(command);
        let Some((w, h)) = self.pty else {
            // no terminal: the page as plain text
            tokio::spawn(async move {
                let _ = handle.data(ch, app::plain(page).into_bytes()).await;
                finish(&handle, ch).await;
            });
            return;
        };
        let (tx, rx) = mpsc::channel(256);
        self.input = Some(tx);
        let shared = self.shared.clone();
        let key = self.key.clone();
        tokio::spawn(async move {
            if shared.sessions.fetch_add(1, Ordering::SeqCst) >= MAX_SESSIONS {
                shared.sessions.fetch_sub(1, Ordering::SeqCst);
                let msg = "xorwyrm.sh is busy right now. try again in a bit, or visit https://xorwyrm.sh\r\n";
                let _ = handle.data(ch, msg.as_bytes().to_vec()).await;
                finish(&handle, ch).await;
                return;
            }
            run(&handle, ch, rx, (w, h), page, &shared, key).await;
            shared.sessions.fetch_sub(1, Ordering::SeqCst);
            finish(&handle, ch).await;
        });
    }
}

async fn finish(handle: &Handle, ch: ChannelId) {
    let _ = handle.exit_status_request(ch, 0).await;
    let _ = handle.eof(ch).await;
    let _ = handle.close(ch).await;
}

impl server::Handler for Conn {
    type Error = russh::Error;

    async fn auth_publickey(&mut self, _: &str, key: VerifiedPublicKey<'_>) -> Result<Auth, Self::Error> {
        self.key = Some(key.public_key.fingerprint(HashAlg::Sha256).to_string());
        Ok(Auth::Accept)
    }

    // no key: let them in without asking anything
    async fn auth_keyboard_interactive<'a>(
        &'a mut self,
        _: &str,
        _: &str,
        _: Option<server::Response<'a>>,
    ) -> Result<Auth, Self::Error> {
        Ok(Auth::Accept)
    }

    async fn channel_open_session(
        &mut self,
        _: Channel<Msg>,
        reply: server::ChannelOpenHandle,
        _: &mut Session,
    ) -> Result<(), Self::Error> {
        // one session per connection is plenty
        if self.input.is_none() {
            reply.accept().await;
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    async fn pty_request(
        &mut self,
        ch: ChannelId,
        _: &str,
        w: u32,
        h: u32,
        _: u32,
        _: u32,
        _: &[(russh::Pty, u32)],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.pty = Some((w, h));
        session.channel_success(ch)
    }

    async fn shell_request(&mut self, ch: ChannelId, session: &mut Session) -> Result<(), Self::Error> {
        session.channel_success(ch)?;
        self.start(ch, "", session);
        Ok(())
    }

    async fn exec_request(&mut self, ch: ChannelId, data: &[u8], session: &mut Session) -> Result<(), Self::Error> {
        session.channel_success(ch)?;
        let command = String::from_utf8_lossy(data).chars().take(64).collect::<String>();
        self.start(ch, &command, session);
        Ok(())
    }

    async fn subsystem_request(&mut self, ch: ChannelId, _: &str, session: &mut Session) -> Result<(), Self::Error> {
        // no sftp or anything else
        session.channel_failure(ch)
    }

    async fn data(&mut self, _: ChannelId, data: &[u8], _: &mut Session) -> Result<(), Self::Error> {
        self.send(Input::Data(data.to_vec()));
        Ok(())
    }

    async fn window_change_request(
        &mut self,
        _: ChannelId,
        w: u32,
        h: u32,
        _: u32,
        _: u32,
        _: &mut Session,
    ) -> Result<(), Self::Error> {
        self.send(Input::Resize(w, h));
        Ok(())
    }

    async fn channel_eof(&mut self, _: ChannelId, _: &mut Session) -> Result<(), Self::Error> {
        self.send(Input::Close);
        Ok(())
    }

    async fn channel_close(&mut self, _: ChannelId, _: &mut Session) -> Result<(), Self::Error> {
        self.send(Input::Close);
        Ok(())
    }
}

/// Terminal sizes are clamped to something sane before anything is drawn.
fn size(w: u32, h: u32) -> (i32, i32) {
    (w.clamp(10, 500) as i32, h.clamp(5, 200) as i32)
}

async fn run(
    handle: &Handle,
    ch: ChannelId,
    mut rx: mpsc::Receiver<Input>,
    (w, h): (u32, u32),
    page: Page,
    shared: &Shared,
    key: Option<String>,
) {
    let remembered = key.as_ref().and_then(|k| shared.names.lock().unwrap().get(k).copied());
    let (w, h) = size(w, h);
    let mut app = App::new(w, h, page, remembered);
    let mut screen = term::Screen::new();
    let mut parser = input::Parser::default();
    // the other screen (so the terminal comes back as it was), no cursor, no line wrap, the
    // mouse reported everywhere it moves (SGR format), and a window title
    let mut out: Vec<u8> = b"\x1b[?1049h\x1b[?25l\x1b[?7l\x1b[?1003h\x1b[?1006h\x1b]0;xorwyrm.sh\x07".to_vec();
    let mut tick = tokio::time::interval(FRAME);
    tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let mut last = Instant::now();
    let mut last_input = Instant::now();
    let mut idle = false;

    loop {
        tokio::select! {
            _ = tick.tick() => {
                let now = Instant::now();
                app.tick((now - last).as_secs_f32().min(0.1));
                last = now;
                if last_input.elapsed() > IDLE_LIMIT {
                    idle = true;
                    break;
                }
                let frame = app.draw();
                out.extend(app.take_output());
                screen.draw(&frame, &mut out);
                if let (Some(n), Some(k)) = (app.take_name(), &key) {
                    let mut names = shared.names.lock().unwrap();
                    if names.len() >= MAX_NAMES && !names.contains_key(k) {
                        names.clear();
                    }
                    names.insert(k.clone(), n);
                }
                // waits while the client catches up, so a slow link gets fewer frames
                if !out.is_empty() && handle.data(ch, std::mem::take(&mut out)).await.is_err() {
                    return;
                }
            }
            msg = rx.recv() => match msg {
                None | Some(Input::Close) => return,
                Some(Input::Data(d)) => {
                    last_input = Instant::now();
                    for ev in parser.feed(&d) {
                        app.input(ev);
                    }
                    if app.quit {
                        break;
                    }
                }
                Some(Input::Resize(w, h)) => {
                    let (w, h) = size(w, h);
                    app.resize(w, h);
                    screen.invalidate();
                }
            }
        }
    }
    let mut bye = b"\x1b[?1003l\x1b[?1006l\x1b[?7h\x1b[0m\x1b[?25h\x1b[?1049l".to_vec();
    if idle {
        bye.extend_from_slice(b"closed after 30 minutes idle. come back any time\r\n");
    }
    let _ = handle.data(ch, bye).await;
}

fn host_key() -> PrivateKey {
    let path = std::env::var("XORWYRM_HOST_KEY").unwrap_or_else(|_| "host_key".into());
    if std::path::Path::new(&path).exists() {
        return PrivateKey::read_openssh_file(&path).unwrap_or_else(|e| panic!("can't read the host key at {path}: {e}"));
    }
    // made once and kept: if it changed, returning visitors would get a scary warning
    let key = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).expect("can't make a host key");
    key.write_openssh_file(&path, LineEnding::LF).unwrap_or_else(|e| panic!("can't save the host key to {path}: {e}"));
    eprintln!("made a new host key at {path}");
    key
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("snapshot") {
        return snapshot(&args[1..]);
    }

    let listen = std::env::var("XORWYRM_LISTEN").unwrap_or_else(|_| "0.0.0.0:2222".into());
    let config = Arc::new(server::Config {
        server_id: SshId::Standard("SSH-2.0-xorwyrm.sh".into()),
        methods: MethodSet::from(&[MethodKind::PublicKey, MethodKind::KeyboardInteractive][..]),
        auth_rejection_time: Duration::from_secs(1),
        // OpenSSH opens with a "none" attempt to see what's on offer: no need to make it wait
        auth_rejection_time_initial: Some(Duration::ZERO),
        keys: vec![host_key()],
        // sessions end on their own when idle (IDLE_LIMIT); keepalives find dead connections
        inactivity_timeout: None,
        keepalive_interval: Some(Duration::from_secs(30)),
        keepalive_max: 4,
        nodelay: true,
        ..Default::default()
    });
    let mut server =
        Server { shared: Arc::new(Shared { names: Mutex::new(HashMap::new()), sessions: AtomicUsize::new(0) }) };
    eprintln!("listening on {listen}");
    server.run_on_address(config, listen.as_str()).await.expect("server stopped");
}

/// `snapshot <page> <cols> <rows> <seconds> [x y]`: renders the app as it would look after that
/// many seconds, with the mouse at x, y, as an HTML page on stdout. For checking the look
/// without an ssh client.
fn snapshot(args: &[String]) {
    let arg = |i: usize, d: f32| args.get(i).and_then(|a| a.parse().ok()).unwrap_or(d);
    let page = Page::from_path(args.first().map_or("", String::as_str));
    let (w, h) = (arg(1, 120.0) as i32, arg(2, 36.0) as i32);
    let mut app = App::new(w, h, page, None);
    let frames = (arg(3, 3.0) / FRAME.as_secs_f32()) as usize;
    let mouse = args.get(5).map(|_| (arg(4, 0.0) as i32, arg(5, 0.0) as i32));
    for _ in 0..frames {
        if let Some((x, y)) = mouse {
            app.input(input::Event::Mouse(input::Mouse::Move, x, y));
        }
        app.tick(FRAME.as_secs_f32());
    }
    let buf = app.draw();
    let mut html = String::from(
        "<!doctype html><meta charset=utf-8><style>body{margin:0;background:#0d0b1c}\
         pre{margin:0;font:16px/1 'Cascadia Mono',Menlo,Consolas,monospace;letter-spacing:0}\
         span{display:inline-block;width:1ch;height:2ch;line-height:2ch;vertical-align:top;overflow:hidden}</style><pre>",
    );
    for y in 0..buf.h {
        for x in 0..buf.w {
            let c = buf.get(x, y).unwrap();
            if c.ch == '\0' {
                continue;
            }
            let (f, b) = (c.fg, c.bg);
            // half blocks as gradients, so they don't depend on the font's block glyphs
            let (ch, bg) = match c.ch {
                '▀' => (' ', format!("linear-gradient(rgb({},{},{}) 50%,rgb({},{},{}) 50%)", f.0, f.1, f.2, b.0, b.1, b.2)),
                '▄' => (' ', format!("linear-gradient(rgb({},{},{}) 50%,rgb({},{},{}) 50%)", b.0, b.1, b.2, f.0, f.1, f.2)),
                ch => (ch, format!("rgb({},{},{})", b.0, b.1, b.2)),
            };
            let esc = match ch {
                '<' => "&lt;".to_string(),
                '&' => "&amp;".to_string(),
                ch => ch.to_string(),
            };
            html.push_str(&format!(
                "<span style=\"color:rgb({},{},{});background:{bg}{}{}\">{esc}</span>",
                f.0,
                f.1,
                f.2,
                if c.bold { ";font-weight:bold" } else { "" },
                if c.ul { ";text-decoration:underline" } else { "" },
            ));
        }
        html.push('\n');
    }
    html.push_str("</pre>");
    println!("{html}");
}
