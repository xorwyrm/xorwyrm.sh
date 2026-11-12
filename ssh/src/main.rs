// xorwyrm.sh over ssh: `ssh term.xorwyrm.sh` opens the site in your terminal.
//
// An ssh server that runs no shell and no commands. Every session gets the site, drawn by
// app.rs. Anyone can log in: any key, or none (keyboard-interactive with no questions). The
// key, when there is one, is only used to remember whether you left the name as letters or
// binary, in memory, until the server restarts.
//
// `ssh -t term.xorwyrm.sh things` opens a page; without a terminal (`ssh term.xorwyrm.sh links`, or
// piped) it prints the page as plain text.
//
// It faces the whole internet on port 22, so everything a visitor can make it do is bounded:
// how many connections there are (and how many from one address), how long one can go without
// opening the site, one session per connection, how fast input is read and how much of it is
// looked at per frame, how much is sent (per session and in all), and how long a frame may
// wait on a client that has stopped reading. Nothing a visitor sends is ever run, written to
// disk or echoed back.
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
use std::future::Future;
use std::io;
use std::net::{IpAddr, Ipv6Addr};
use std::pin::Pin;
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use russh::keys::ssh_key::LineEnding;
use russh::keys::{Algorithm, HashAlg, PrivateKey};
use russh::server::{self, Auth, Handle, Msg, Session, VerifiedPublicKey};
use russh::{Channel, ChannelId, MethodKind, MethodSet, Preferred, SshId, kex};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Notify, mpsc, oneshot};
use tokio::time::MissedTickBehavior;

use app::{App, Page};

/// Connections at once, counting ones that haven't opened the site yet. Past this, new ones
/// are closed straight away. Well under the 1024 files the process may have open.
const MAX_CONNS: usize = 320;
/// Connections at once from one address (an IPv4 address, or an IPv6 /48).
const MAX_PER_ADDR: usize = 8;
/// Sessions at once; past this, newcomers are asked to come back later.
const MAX_SESSIONS: usize = 200;
/// A connection has this long to open the site (time to answer the host key question), and
/// is dropped if it goes this long without sending anything or taking what it's sent.
const LOGIN_LIMIT: Duration = Duration::from_secs(120);
/// A session ends after this long without a key press or mouse move.
const IDLE_LIMIT: Duration = Duration::from_secs(30 * 60);
/// A session ends if a frame can't be handed over for this long: the client stopped reading.
const STALL_LIMIT: Duration = Duration::from_secs(30);
/// Once a session is over the client has this long to hang up before it's hung up on.
const CLOSE_GRACE: Duration = Duration::from_secs(3);
/// About 25 frames a second.
const FRAME: Duration = Duration::from_millis(40);
/// Keys and mouse moves handled per frame: more than a person can make. The rest are dropped,
/// so a flood of input can't keep the server busy.
const EVENTS_PER_FRAME: usize = 64;
/// Input is taken in pieces of at most this many bytes, and this many pieces may wait.
const INPUT_PIECE: usize = 4096;
const INPUT_QUEUE: usize = 64;
/// A connection is read from at this many bytes a second at most, after a first allowance.
/// Keys and mouse moves come to a few thousand a second at the very most; anything faster is
/// a flood, and it waits in the sender's own queue instead of costing work here.
const IN_RATE: f64 = 64_000.0;
const IN_BURST: f64 = 256_000.0;
/// When the server is busy, a session is sent this many bytes a second at most, after a first
/// allowance (a whole big screen): very big windows get fewer frames then. On a quiet server
/// every window draws at full speed.
const OUT_RATE: f64 = 1_500_000.0;
const OUT_BURST: f64 = 4_000_000.0;
/// And all sessions together this many (about 190 Mbit/s): when it runs short everyone gets
/// fewer frames, and the rest of the machine keeps its network.
const ALL_OUT_RATE: i64 = 24_000_000;
/// How many remembered visitors to keep.
const MAX_NAMES: usize = 100_000;

/// A lock that still opens if a thread panicked while holding it.
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

#[derive(Default)]
struct Conns {
    total: usize,
    by_addr: HashMap<IpAddr, usize>,
}

struct Shared {
    /// by key fingerprint: whether the name was left decoded
    names: Mutex<HashMap<String, bool>>,
    sessions: AtomicUsize,
    conns: Mutex<Conns>,
    /// bytes all sessions may still send, topped up as time passes; frames wait while it's
    /// below zero
    out: AtomicI64,
    // since the last hourly log line
    accepted: AtomicUsize,
    refused: AtomicUsize,
}

/// What connections are counted under: an IPv4 address as it is, an IPv6 address by its /48,
/// since whoever has one address there usually has the whole block.
fn bucket(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V4(_) => ip,
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => IpAddr::V4(v4),
            None => {
                let s = v6.segments();
                IpAddr::V6(Ipv6Addr::new(s[0], s[1], s[2], 0, 0, 0, 0, 0))
            }
        },
    }
}

/// A place among the connections, given back when dropped.
struct Slot {
    shared: Arc<Shared>,
    addr: IpAddr,
}

impl Shared {
    fn new() -> Arc<Shared> {
        Arc::new(Shared {
            names: Mutex::new(HashMap::new()),
            sessions: AtomicUsize::new(0),
            conns: Mutex::new(Conns::default()),
            out: AtomicI64::new(ALL_OUT_RATE),
            accepted: AtomicUsize::new(0),
            refused: AtomicUsize::new(0),
        })
    }

    fn admit(self: &Arc<Self>, ip: IpAddr) -> Option<Slot> {
        let addr = bucket(ip);
        let mut c = lock(&self.conns);
        if c.total >= MAX_CONNS || c.by_addr.get(&addr).is_some_and(|&n| n >= MAX_PER_ADDR) {
            return None;
        }
        c.total += 1;
        *c.by_addr.entry(addr).or_insert(0) += 1;
        Some(Slot { shared: self.clone(), addr })
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        let mut c = lock(&self.shared.conns);
        c.total = c.total.saturating_sub(1);
        if let Some(n) = c.by_addr.get_mut(&self.addr) {
            *n = n.saturating_sub(1);
            if *n == 0 {
                c.by_addr.remove(&self.addr);
            }
        }
    }
}

/// How far along a connection is, for the task that decides when to drop it.
#[derive(Default)]
struct Life {
    /// the site was opened
    started: Notify,
    /// and has ended
    done: Notify,
}

/// Marks a session over when dropped, however it ends (a panic included), and gives back its
/// place among the sessions if it had one.
struct Ending {
    life: Arc<Life>,
    counted: Option<Arc<Shared>>,
}

impl Drop for Ending {
    fn drop(&mut self) {
        if let Some(shared) = &self.counted {
            shared.sessions.fetch_sub(1, Ordering::SeqCst);
        }
        self.life.done.notify_one();
    }
}

/// The socket, with a way to cut it from outside: once `kill` fires (or its sender is gone)
/// every read and write fails, which ends the connection whatever state it's in. It also
/// reads no faster than IN_RATE.
struct Guarded {
    inner: TcpStream,
    kill: oneshot::Receiver<()>,
    dead: bool,
    /// bytes that may be read now
    allowance: f64,
    topped_up: tokio::time::Instant,
    wait: Pin<Box<tokio::time::Sleep>>,
}

impl Guarded {
    fn new(inner: TcpStream, kill: oneshot::Receiver<()>) -> Guarded {
        Guarded {
            inner,
            kill,
            dead: false,
            allowance: IN_BURST,
            topped_up: tokio::time::Instant::now(),
            wait: Box::pin(tokio::time::sleep(Duration::ZERO)),
        }
    }

    fn check(&mut self, cx: &mut Context<'_>) -> io::Result<()> {
        if !self.dead && Pin::new(&mut self.kill).poll(cx).is_ready() {
            self.dead = true;
        }
        if self.dead { Err(io::ErrorKind::ConnectionAborted.into()) } else { Ok(()) }
    }
}

impl AsyncRead for Guarded {
    fn poll_read(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<io::Result<()>> {
        let me = &mut *self;
        if let Err(e) = me.check(cx) {
            return Poll::Ready(Err(e));
        }
        if buf.remaining() == 0 {
            return Poll::Ready(Ok(()));
        }
        loop {
            let now = tokio::time::Instant::now();
            me.allowance = (me.allowance + (now - me.topped_up).as_secs_f64() * IN_RATE).min(IN_BURST);
            me.topped_up = now;
            if me.allowance >= 1.0 {
                break;
            }
            // used up: leave the rest unread for a moment. it backs up to the sender, who has
            // to slow down
            me.wait.as_mut().reset(now + Duration::from_millis(100));
            if me.wait.as_mut().poll(cx).is_pending() {
                return Poll::Pending;
            }
        }
        let room = (me.allowance as usize).min(buf.remaining());
        let mut part = ReadBuf::new(buf.initialize_unfilled_to(room));
        match Pin::new(&mut me.inner).poll_read(cx, &mut part) {
            Poll::Ready(Ok(())) => {
                let n = part.filled().len();
                buf.advance(n);
                me.allowance -= n as f64;
                Poll::Ready(Ok(()))
            }
            other => other,
        }
    }
}

impl AsyncWrite for Guarded {
    fn poll_write(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<io::Result<usize>> {
        if let Err(e) = self.check(cx) {
            return Poll::Ready(Err(e));
        }
        Pin::new(&mut self.inner).poll_write(cx, buf)
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        if let Err(e) = self.check(cx) {
            return Poll::Ready(Err(e));
        }
        Pin::new(&mut self.inner).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        if let Err(e) = self.check(cx) {
            return Poll::Ready(Err(e));
        }
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

/// One connection, from accepting it to dropping it. It's dropped when it ends by itself,
/// when it hasn't opened the site within LOGIN_LIMIT, or shortly after its session is over.
async fn serve(config: Arc<server::Config>, socket: TcpStream, shared: Arc<Shared>, _slot: Slot) {
    let _ = socket.set_nodelay(true);
    let life = Arc::new(Life::default());
    let (kill, killed) = oneshot::channel();
    let conn = Conn { shared, life: life.clone(), key: None, chan: None, pty: None, started: false, input: None };
    let stream = Guarded::new(socket, killed);
    let work = async move {
        if let Ok(session) = server::run_stream(config, stream, conn).await {
            let _ = session.await;
        }
    };
    let over = async {
        if tokio::time::timeout(LOGIN_LIMIT, life.started.notified()).await.is_ok() {
            life.done.notified().await;
            tokio::time::sleep(CLOSE_GRACE).await;
        }
    };
    tokio::select! {
        _ = work => {}
        _ = over => {}
    }
    // the connection runs in a task of its own by now; this is what stops it
    let _ = kill.send(());
}

enum Input {
    Data(Vec<u8>),
    Resize(u32, u32),
    Close,
}

/// One ssh connection.
struct Conn {
    shared: Arc<Shared>,
    life: Arc<Life>,
    key: Option<String>,
    /// the one channel a connection gets
    chan: Option<ChannelId>,
    /// columns and rows, once a terminal is asked for
    pty: Option<(u32, u32)>,
    /// the site was opened: it's opened once
    started: bool,
    input: Option<mpsc::Sender<Input>>,
}

impl Conn {
    fn send(&self, i: Input) {
        if let Some(tx) = &self.input {
            // a full queue means the session is stuck or flooded: drop the input
            let _ = tx.try_send(i);
        }
    }

    /// Whether a request to open the site on this channel is allowed: on the connection's
    /// one channel, once.
    fn may_start(&self, ch: ChannelId) -> bool {
        !self.started && self.chan == Some(ch)
    }

    fn start(&mut self, ch: ChannelId, command: &str, session: &mut Session) {
        self.started = true;
        self.life.started.notify_one();
        let handle = session.handle();
        let page = Page::from_path(command);
        let life = self.life.clone();
        let Some((w, h)) = self.pty else {
            // no terminal: the page as plain text. a page that isn't there fails, so a script
            // that meant to reach some other machine doesn't take this for success
            let status = if page == Page::NotFound { 127 } else { 0 };
            tokio::spawn(async move {
                let _ending = Ending { life, counted: None };
                let _ = tokio::time::timeout(STALL_LIMIT, async {
                    let _ = handle.data(ch, app::plain(page).into_bytes()).await;
                    finish(&handle, ch, status).await;
                })
                .await;
            });
            return;
        };
        let (tx, rx) = mpsc::channel(INPUT_QUEUE);
        self.input = Some(tx);
        let shared = self.shared.clone();
        let key = self.key.clone();
        tokio::spawn(async move {
            if shared.sessions.fetch_add(1, Ordering::SeqCst) >= MAX_SESSIONS {
                shared.sessions.fetch_sub(1, Ordering::SeqCst);
                let _ending = Ending { life, counted: None };
                let msg = "xorwyrm.sh is busy right now. try again in a bit, or visit https://xorwyrm.sh\r\n";
                let _ = tokio::time::timeout(STALL_LIMIT, async {
                    let _ = handle.data(ch, msg.as_bytes().to_vec()).await;
                    finish(&handle, ch, 1).await;
                })
                .await;
                return;
            }
            let _ending = Ending { life, counted: Some(shared.clone()) };
            run(&handle, ch, rx, (w, h), page, &shared, key).await;
            let _ = tokio::time::timeout(CLOSE_GRACE, finish(&handle, ch, 0)).await;
        });
    }
}

async fn finish(handle: &Handle, ch: ChannelId, status: u32) {
    let _ = handle.exit_status_request(ch, status).await;
    let _ = handle.eof(ch).await;
    let _ = handle.close(ch).await;
}

// Anything not handled here is refused by the library: passwords, port forwarding in either
// direction, X11, agent forwarding. There's no way through this server to anything else.
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
        channel: Channel<Msg>,
        reply: server::ChannelOpenHandle,
        _: &mut Session,
    ) -> Result<(), Self::Error> {
        // one channel per connection, ever; not accepting refuses it
        if self.chan.is_none() {
            self.chan = Some(channel.id());
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
        if !self.may_start(ch) {
            return session.channel_failure(ch);
        }
        self.pty = Some((w, h));
        session.channel_success(ch)
    }

    async fn shell_request(&mut self, ch: ChannelId, session: &mut Session) -> Result<(), Self::Error> {
        if !self.may_start(ch) {
            return session.channel_failure(ch);
        }
        session.channel_success(ch)?;
        self.start(ch, "", session);
        Ok(())
    }

    // The "command" is never run: it's only compared with the pages' names.
    async fn exec_request(&mut self, ch: ChannelId, data: &[u8], session: &mut Session) -> Result<(), Self::Error> {
        if !self.may_start(ch) {
            return session.channel_failure(ch);
        }
        session.channel_success(ch)?;
        let command = String::from_utf8_lossy(&data[..data.len().min(256)]).chars().take(64).collect::<String>();
        self.start(ch, &command, session);
        Ok(())
    }

    async fn subsystem_request(&mut self, ch: ChannelId, _: &str, session: &mut Session) -> Result<(), Self::Error> {
        // no sftp or anything else
        session.channel_failure(ch)
    }

    async fn data(&mut self, _: ChannelId, data: &[u8], _: &mut Session) -> Result<(), Self::Error> {
        self.send(Input::Data(data[..data.len().min(INPUT_PIECE)].to_vec()));
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
    let remembered = key.as_ref().and_then(|k| lock(&shared.names).get(k).copied());
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
    let mut budget = EVENTS_PER_FRAME;
    // the newest window size waits here for the next frame, so a flood of them costs nothing
    let mut resized: Option<(u32, u32)> = None;
    // bytes this session may still send; it can overdraw, and then frames wait
    let mut allowance = OUT_BURST;

    loop {
        tokio::select! {
            _ = tick.tick() => {
                let now = Instant::now();
                let dt = (now - last).as_secs_f32();
                app.tick(dt.min(0.25));
                last = now;
                if last_input.elapsed() > IDLE_LIMIT {
                    idle = true;
                    break;
                }
                budget = EVENTS_PER_FRAME;
                // A frame is only drawn when there's allowance for it. Everyone's allowance is
                // the hard limit. A session's own share only counts once that's half used:
                // on a quiet server a big window draws at full speed, and on a busy one it
                // gets fewer frames so the rest keep theirs. It can owe a second at most, so
                // it isn't shut out for long when things get busy.
                allowance = (allowance + dt as f64 * OUT_RATE).clamp(-OUT_RATE, OUT_BURST);
                let left = shared.out.load(Ordering::Relaxed);
                if left < 0 || (allowance < 0.0 && left < ALL_OUT_RATE / 2) {
                    continue;
                }
                if let Some((w, h)) = resized.take() {
                    let (w, h) = size(w, h);
                    app.resize(w, h);
                    screen.invalidate();
                }
                let frame = app.draw();
                out.extend(app.take_output());
                screen.draw(&frame, &mut out);
                if let (Some(n), Some(k)) = (app.take_name(), &key) {
                    let mut names = lock(&shared.names);
                    if names.len() >= MAX_NAMES && !names.contains_key(k) {
                        names.clear();
                    }
                    names.insert(k.clone(), n);
                }
                // waits while the client catches up, so a slow link gets fewer frames; a client
                // that has stopped taking them altogether is let go
                if !out.is_empty() {
                    allowance -= out.len() as f64;
                    shared.out.fetch_sub(out.len() as i64, Ordering::Relaxed);
                    let sent = tokio::time::timeout(STALL_LIMIT, handle.data(ch, std::mem::take(&mut out))).await;
                    if !matches!(sent, Ok(Ok(()))) {
                        return;
                    }
                }
            }
            msg = rx.recv() => match msg {
                None | Some(Input::Close) => return,
                Some(Input::Data(d)) => {
                    last_input = Instant::now();
                    for ev in parser.feed(&d) {
                        if budget == 0 {
                            break;
                        }
                        budget -= 1;
                        app.input(ev);
                    }
                    if app.quit {
                        break;
                    }
                }
                Some(Input::Resize(w, h)) => resized = Some((w, h)),
            }
        }
    }
    let mut bye = b"\x1b[?1003l\x1b[?1006l\x1b[?7h\x1b[0m\x1b[?25h\x1b[?1049l".to_vec();
    if idle {
        bye.extend_from_slice(b"closed after 30 minutes idle. come back any time\r\n");
    }
    let _ = tokio::time::timeout(CLOSE_GRACE, handle.data(ch, bye)).await;
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

fn config() -> server::Config {
    server::Config {
        server_id: SshId::Standard("SSH-2.0-xorwyrm.sh".into()),
        methods: MethodSet::from(&[MethodKind::PublicKey, MethodKind::KeyboardInteractive][..]),
        auth_rejection_time: Duration::from_secs(1),
        // OpenSSH opens with a "none" attempt to see what's on offer: no need to make it wait
        auth_rejection_time_initial: Some(Duration::ZERO),
        max_auth_attempts: 6,
        keys: vec![host_key()],
        // Only the curve25519 key exchanges, which every client from the last ten years has.
        // The classic Diffie-Hellman ones cost the server real work per connection, before
        // the client has done anything, and are left out.
        preferred: Preferred {
            kex: (&[
                kex::MLKEM768X25519_SHA256,
                kex::CURVE25519,
                kex::CURVE25519_PRE_RFC_8731,
                kex::EXTENSION_SUPPORT_AS_SERVER,
                kex::EXTENSION_OPENSSH_STRICT_KEX_AS_SERVER,
            ][..])
                .into(),
            ..Default::default()
        },
        // A connection that sends nothing, or stops taking what it's sent, is dropped. One
        // that's only watching still answers the keepalives, which counts.
        inactivity_timeout: Some(LOGIN_LIMIT),
        keepalive_interval: Some(Duration::from_secs(30)),
        keepalive_max: 3,
        ..Default::default()
    }
}

// A few threads are plenty, and it's kept to a small share of the machine anyway.
#[tokio::main(worker_threads = 4)]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("snapshot") {
        return snapshot(&args[1..]);
    }

    let listen = std::env::var("XORWYRM_LISTEN").unwrap_or_else(|_| "0.0.0.0:2222".into());
    let config = Arc::new(config());
    let shared = Shared::new();
    let listener = TcpListener::bind(listen.as_str()).await.unwrap_or_else(|e| panic!("can't listen on {listen}: {e}"));
    eprintln!("listening on {listen}");

    // Everyone's sending allowance, topped up twenty times a second, never past a second's worth.
    let all = shared.clone();
    tokio::spawn(async move {
        let mut step = tokio::time::interval(Duration::from_millis(50));
        step.set_missed_tick_behavior(MissedTickBehavior::Skip);
        loop {
            step.tick().await;
            // only this task adds, so reading first can't overshoot
            let room = ALL_OUT_RATE - all.out.load(Ordering::Relaxed);
            all.out.fetch_add((ALL_OUT_RATE / 20).min(room).max(0), Ordering::Relaxed);
        }
    });

    // One line an hour, when there's been anything: port 22 is knocked on all day, and a line
    // per knock would only bury this.
    let counts = shared.clone();
    tokio::spawn(async move {
        let mut hour = tokio::time::interval(Duration::from_secs(3600));
        hour.tick().await;
        loop {
            hour.tick().await;
            let (a, r) = (counts.accepted.swap(0, Ordering::Relaxed), counts.refused.swap(0, Ordering::Relaxed));
            if a + r > 0 {
                let open = lock(&counts.conns).total;
                let sessions = counts.sessions.load(Ordering::Relaxed);
                eprintln!("last hour: {a} connections, {r} turned away at the limit; now {open} open, {sessions} sessions");
            }
        }
    });

    loop {
        let (socket, peer) = match listener.accept().await {
            Ok(c) => c,
            Err(_) => {
                // out of files or memory for a moment: wait, don't fall over
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            }
        };
        match shared.admit(peer.ip()) {
            Some(slot) => {
                shared.accepted.fetch_add(1, Ordering::Relaxed);
                tokio::spawn(serve(config.clone(), socket, shared.clone(), slot));
            }
            // dropping the socket closes it
            None => {
                shared.refused.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
}

/// `snapshot <page> <cols> <rows> <seconds> [x y]`: renders the app as it would look after that
/// many seconds, with the mouse at x, y, as an HTML page on stdout. For checking the look
/// without an ssh client.
fn snapshot(args: &[String]) {
    let arg = |i: usize, d: f32| args.get(i).and_then(|a| a.parse().ok()).unwrap_or(d);
    let page = Page::from_path(args.first().map_or("", String::as_str));
    let (w, h) = size(arg(1, 120.0) as u32, arg(2, 36.0) as u32);
    let mut app = App::new(w, h, page, None);
    let frames = (arg(3, 3.0).clamp(0.0, 600.0) / FRAME.as_secs_f32()) as usize;
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
            let Some(c) = buf.get(x, y) else { continue };
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connections_are_counted_per_address() {
        let shared = Shared::new();
        let a: IpAddr = "203.0.113.7".parse().unwrap();
        let slots: Vec<Slot> = (0..MAX_PER_ADDR).map(|_| shared.admit(a).expect("under the limit")).collect();
        assert!(shared.admit(a).is_none(), "one address can't take more than its share");
        // the same address arriving on the IPv6 socket is the same address
        assert!(shared.admit("::ffff:203.0.113.7".parse().unwrap()).is_none());
        assert!(shared.admit("203.0.113.8".parse().unwrap()).is_some());
        drop(slots);
        assert!(shared.admit(a).is_some(), "places come back when connections end");
        assert!(lock(&shared.conns).by_addr.is_empty(), "and nothing is left behind");
    }

    #[test]
    fn ipv6_is_counted_by_block() {
        let shared = Shared::new();
        let slots: Vec<Slot> = (0..MAX_PER_ADDR)
            .map(|i| shared.admit(format!("2001:db8:1:{i}::1").parse().unwrap()).expect("under the limit"))
            .collect();
        assert!(shared.admit("2001:db8:1:ffff::9".parse().unwrap()).is_none());
        assert!(shared.admit("2001:db8:2::1".parse().unwrap()).is_some());
        drop(slots);
    }

    #[test]
    fn there_is_a_limit_on_everyone_together() {
        let shared = Shared::new();
        let slots: Vec<Slot> =
            (0..MAX_CONNS).map(|i| shared.admit(format!("10.{}.{}.1", i / 256, i % 256).parse().unwrap()).unwrap()).collect();
        assert!(shared.admit("192.0.2.1".parse().unwrap()).is_none());
        drop(slots);
        assert_eq!(lock(&shared.conns).total, 0);
    }
}
