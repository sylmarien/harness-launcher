//! The tmux control-mode client every spawn's output arrives down.
//!
//! One client carries every pane in the session, so a single reader serves
//! twenty spawns. A control client needs a pty of its own: on piped stdio tmux
//! refuses with `tcgetattr failed`. It streams only what is produced while it
//! is attached, so the app attaches before starting anything. Everything else
//! tmux is asked goes the other way, as ordinary commands; only keystrokes and
//! the size of the slot travel back up this client.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::Duration;

use portable_pty::{CommandBuilder, MasterPty, PtySize, native_pty_system};

use crate::error::{Error, Result};
use crate::screen::{Screen, Size};
use crate::tmux::Server;

/// How long the app waits to hear whether a control client attached.
const ATTACHING: Duration = Duration::from_secs(5);

/// The grid behind one pane, shared between the reader and whatever draws it.
pub type Grid = Arc<Mutex<Screen>>;

/// Which grid each pane's output belongs to.
type Grids = Arc<Mutex<HashMap<String, Grid>>>;

/// An attached control-mode client.
///
/// Dropping it hangs the client up and takes nothing with it: the session and
/// its spawns belong to the tmux server, which outlives this process.
pub struct Client {
    /// The pty the client runs in; closing it is what ends the client.
    terminal: Box<dyn MasterPty + Send>,
    /// What the app says back: keystrokes, and the size of the slot.
    saying: Arc<Mutex<Box<dyn Write + Send>>>,
    /// The grids output is routed into.
    grids: Grids,
    /// Why the reader stopped, once it has.
    ended: Ended,
}

/// Why the reader stopped, once it has.
///
/// A dead client leaves the grids frozen rather than visibly failed, so the
/// reader records its reason and the app checks for one every frame.
type Ended = Arc<OnceLock<String>>;

impl Client {
    /// Attach to a session, and start reading everything in it.
    ///
    /// The size is the slot's: a control client's size shapes every window in
    /// the session.
    pub fn attach(server: &Server, session: &str, slot: Size) -> Result<Self> {
        let pty = native_pty_system()
            .openpty(sized(slot))
            .map_err(|trouble| {
                Error::new(format!(
                    "the app could not open a terminal for the control-mode client: {trouble}"
                ))
            })?;

        let mut client = CommandBuilder::new("tmux");
        client.args(["-L", server.socket(), "-CC", "attach", "-t", session]);
        forget_the_users_session(&mut client);
        if std::env::var_os("TERM").is_none() {
            client.env("TERM", "xterm-256color");
        }

        pty.slave.spawn_command(client).map_err(|trouble| {
            Error::new(format!(
                "the app could not start a control-mode client: {trouble} — is tmux installed \
                 and on PATH?"
            ))
        })?;
        drop(pty.slave);

        let reader = pty.master.try_clone_reader().map_err(|trouble| {
            Error::new(format!("the control-mode client cannot be read: {trouble}"))
        })?;
        let writer = pty.master.take_writer().map_err(|trouble| {
            Error::new(format!(
                "the control-mode client cannot be written to: {trouble}"
            ))
        })?;

        let grids = Grids::default();
        let ended = Ended::default();
        let (settling, settled) = mpsc::channel();
        let routing = Arc::clone(&grids);
        let reporting = Arc::clone(&ended);
        thread::spawn(move || route(reader, &routing, Handshake::new(settling), &reporting));

        match settled.recv_timeout(ATTACHING) {
            Ok(Attaching::Attached) => {}
            Ok(Attaching::Refused(why)) => {
                return Err(Error::new(format!(
                    "tmux would not attach a control-mode client to `{session}`: {why}"
                )));
            }
            Err(RecvTimeoutError::Disconnected) => {
                return Err(Error::new(format!(
                    "the control-mode client for `{session}` stopped before it said whether it \
                     had attached"
                )));
            }
            Err(RecvTimeoutError::Timeout) => {
                return Err(Error::new(format!(
                    "tmux did not attach a control-mode client to `{session}` within {} seconds, \
                     and said nothing about why",
                    ATTACHING.as_secs()
                )));
            }
        }

        let attached = Self {
            terminal: pty.master,
            saying: Arc::new(Mutex::new(writer)),
            grids,
            ended,
        };
        attached.resize(slot)?;

        Ok(attached)
    }

    /// Put a grid behind a pane, and hand it back.
    ///
    /// Call before starting anything in the pane: output produced with no grid
    /// to route it into is not held anywhere and cannot be asked for again.
    pub fn watch(&self, pane: &str, size: Size) -> Grid {
        let grid: Grid = Arc::new(Mutex::new(Screen::new(size)));
        self.grids
            .lock()
            .expect(POISONED)
            .insert(pane.to_string(), Arc::clone(&grid));

        grid
    }

    /// Let go of a retired pane's grid.
    ///
    /// Output for a pane with no grid is dropped, so a notification still in
    /// flight when this is called is the ordinary case rather than a race.
    pub fn forget(&self, pane: &str) {
        self.grids.lock().expect(POISONED).remove(pane);
    }

    /// Type at a spawn. Bytes travel as hex so nothing has to be quoted for
    /// tmux's parser.
    pub fn send(&self, pane: &str, bytes: &[u8]) -> Result<()> {
        if bytes.is_empty() {
            return Ok(());
        }

        self.say(&send_keys(pane, bytes))
    }

    /// Say how big the slot is now. Windows follow the attached client's size,
    /// so one call resizes every spawn.
    pub fn resize(&self, slot: Size) -> Result<()> {
        self.terminal.resize(sized(slot)).map_err(|trouble| {
            Error::new(format!(
                "the control-mode client would not be resized: {trouble}"
            ))
        })?;

        self.say(&format!("refresh-client -C {}x{}", slot.columns, slot.rows))
    }

    /// Refuse once the client has gone. Asked every frame, because a hung-up
    /// client only leaves grids that have stopped changing.
    pub fn listening(&self) -> Result<()> {
        match self.ended.get() {
            None => Ok(()),
            Some(why) => Err(Error::new(format!(
                "the app is no longer being told what the spawns are drawing: {why}. They are \
                 still running — the tmux server has them"
            ))),
        }
    }

    /// Say one command down the client.
    fn say(&self, command: &str) -> Result<()> {
        let mut saying = self.saying.lock().expect(POISONED);
        writeln!(saying, "{command}")
            .and_then(|()| saying.flush())
            .map_err(|trouble| {
                Error::new(format!(
                    "the control-mode client stopped listening: {trouble}"
                ))
            })
    }
}

/// What a lock says when the thread holding it died mid-write. Said once so
/// the reader and the drawing side report the same fault the same way.
pub const POISONED: &str = "another thread panicked while holding a terminal";

/// Read the client until it hangs up, routing output to the grid that owns it.
///
/// Bytes rather than text: what a spawn draws need not be valid UTF-8, and
/// tmux may chop the stream mid-character.
fn route(reader: Box<dyn Read + Send>, grids: &Grids, mut attaching: Handshake, ended: &Ended) {
    let mut client = BufReader::new(reader);
    let mut said = Vec::new();

    loop {
        said.clear();
        match client.read_until(b'\n', &mut said) {
            Ok(0) => return stopped(ended, "the control-mode client hung up".to_string()),
            Err(trouble) => {
                return stopped(ended, format!("it could not be read any more: {trouble}"));
            }
            Ok(_) => {}
        }
        attaching.read(trimmed(&said));

        let Some(output) = Output::parse(&said) else {
            continue;
        };
        // The map lock is released before the grid is taken, so a slow frame
        // on one grid never holds up another spawn's output.
        let grid = grids.lock().expect(POISONED).get(&output.pane).cloned();
        if let Some(grid) = grid {
            grid.lock().expect(POISONED).apply(&output.bytes);
        }
    }
}

/// Leave behind why the reader stopped; the first reason wins.
fn stopped(ended: &Ended, why: String) {
    let _ = ended.set(why);
}

/// Drop `TMUX`/`TMUX_PANE`, which tmux would read as nesting and refuse.
fn forget_the_users_session(client: &mut CommandBuilder) {
    client.env_remove("TMUX");
    client.env_remove("TMUX_PANE");
}

/// Whether there is a client to talk to.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Attaching {
    Attached,
    /// What tmux said about why not.
    Refused(String),
}

/// Reading the first thing tmux says, which is whether it attached at all.
///
/// A refusal looks exactly like a successful reply until its last line —
/// `%begin`, then `%end` or `%error` — so the verdict waits for that line.
struct Handshake {
    /// Where the answer goes, until there is one.
    settling: Option<Sender<Attaching>>,
    /// What tmux said between the reply starting and it going wrong.
    complaint: Vec<String>,
}

impl Handshake {
    fn new(settling: Sender<Attaching>) -> Self {
        Self {
            settling: Some(settling),
            complaint: Vec::new(),
        }
    }

    /// Read one line, and settle the question if that line settles it.
    fn read(&mut self, line: &[u8]) {
        let Some(settling) = &self.settling else {
            return;
        };
        let said = String::from_utf8_lossy(line);

        // The first `%begin` arrives wrapped in an escape sequence, so markers
        // are looked for anywhere on the line rather than at its start.
        let settled = if said.contains("%end") {
            Attaching::Attached
        } else if said.contains("%error") || said.contains("%exit") {
            Attaching::Refused(self.complaint.join("; "))
        } else {
            if !said.contains("%begin") && !said.trim().is_empty() {
                self.complaint.push(said.trim().to_string());
            }

            return;
        };

        let _ = settling.send(settled);
        self.settling = None;
    }
}

/// What one pane drew, as one `%output` notification said it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Output {
    /// The pane that drew it.
    pub pane: String,
    /// What it drew.
    pub bytes: Vec<u8>,
}

impl Output {
    /// Read one line of the client, if it was `%output`; every other
    /// notification is skipped.
    pub fn parse(line: &[u8]) -> Option<Self> {
        let line = trimmed(line);
        let said = line.strip_prefix(b"%output ")?;
        let space = said.iter().position(|byte| *byte == b' ')?;
        let pane = std::str::from_utf8(&said[..space]).ok()?;

        Some(Self {
            pane: pane.to_string(),
            bytes: unescaped(&said[space + 1..]),
        })
    }
}

/// A line without its terminator. The pty turns `\n` into `\r\n`, and a
/// leftover `\r` would reach a grid as something the spawn drew.
fn trimmed(line: &[u8]) -> &[u8] {
    let mut end = line.len();
    while end > 0 && (line[end - 1] == b'\n' || line[end - 1] == b'\r') {
        end -= 1;
    }

    &line[..end]
}

/// Undo control mode's `\ooo` escaping; everything else passes through.
fn unescaped(said: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(said.len());

    let mut at = 0;
    while at < said.len() {
        if let Some(byte) = octal(said.get(at..at + 4)) {
            bytes.push(byte);
            at += 4;
        } else {
            bytes.push(said[at]);
            at += 1;
        }
    }

    bytes
}

/// One `\ooo` escape, if that is what these four bytes are: exactly three
/// octal digits — the usual parse would also accept a sign or a space.
fn octal(escape: Option<&[u8]>) -> Option<u8> {
    let [b'\\', digits @ ..] = escape? else {
        return None;
    };
    if !digits.iter().all(|digit| (b'0'..=b'7').contains(digit)) {
        return None;
    }

    u8::from_str_radix(std::str::from_utf8(digits).ok()?, 8).ok()
}

/// The `send-keys` that carries a keystroke to a pane.
fn send_keys(pane: &str, bytes: &[u8]) -> String {
    let hex: Vec<String> = bytes.iter().map(|byte| format!("{byte:02x}")).collect();

    format!("send-keys -t {pane} -H {}", hex.join(" "))
}

/// A size, the way a pty is asked for one.
fn sized(size: Size) -> PtySize {
    PtySize {
        rows: size.rows,
        cols: size.columns,
        pixel_width: 0,
        pixel_height: 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tmux::tests::PrivateTmux;
    use std::time::Instant;

    #[test]
    fn output_is_read_as_the_pane_that_drew_it_and_what_it_drew() {
        let output = Output::parse(b"%output %3 hello\r\n").unwrap();

        assert_eq!(output.pane, "%3");
        assert_eq!(output.bytes, b"hello");
    }

    #[test]
    fn everything_a_terminal_could_not_survive_arrives_escaped() {
        let output = Output::parse(b"%output %3 \\033[31mred\\033[0m\\015\\012").unwrap();

        assert_eq!(output.bytes, b"\x1b[31mred\x1b[0m\r\n");
    }

    #[test]
    fn a_backslash_the_spawn_drew_is_not_read_as_an_escape() {
        let output = Output::parse(b"%output %3 C:\\134Users").unwrap();

        assert_eq!(output.bytes, b"C:\\Users");
    }

    #[test]
    fn what_a_spawn_drew_does_not_have_to_be_text() {
        let output = Output::parse(b"%output %3 \xe4\xb8\x96\xe7\x95\x8c \xff\xfe").unwrap();

        assert_eq!(
            output.bytes,
            "世界 "
                .as_bytes()
                .iter()
                .copied()
                .chain([0xff, 0xfe])
                .collect::<Vec<u8>>()
        );
    }

    #[test]
    fn a_trailing_backslash_is_taken_literally_rather_than_read_off_the_end() {
        let output = Output::parse(b"%output %3 ends\\").unwrap();

        assert_eq!(output.bytes, b"ends\\");
    }

    #[test]
    fn everything_that_is_not_output_is_skipped() {
        for line in [
            &b"%window-add @1\r\n"[..],
            b"%begin 1786325706 270 0\r\n",
            b"%layout-change @0 a87d,100x30,0,0,0\r\n",
            b"%exit\r\n",
            b"",
        ] {
            assert_eq!(
                Output::parse(line),
                None,
                "{:?}",
                String::from_utf8_lossy(line)
            );
        }
    }

    #[test]
    fn a_keystroke_travels_as_hex_so_nothing_has_to_be_quoted() {
        assert_eq!(send_keys("%3", b"\x03"), "send-keys -t %3 -H 03");
        assert_eq!(
            send_keys("%3", "ok\r".as_bytes()),
            "send-keys -t %3 -H 6f 6b 0d"
        );
    }

    // The rest drives a real tmux and a real pty on a private socket; a fake
    // would have to pretend about exactly the part under test.

    /// The shape a test's slot is.
    const SLOT: Size = Size {
        columns: 40,
        rows: 10,
    };

    /// What a grid says, with the blanks at the end of each row taken off.
    fn shown(grid: &Grid) -> String {
        crate::screen::tests::shown(&grid.lock().unwrap())
            .iter()
            .map(|row| row.trim_end())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// A spawn that answers what it is told and is still running afterwards.
    ///
    /// The `sleep` is load-bearing: tmux may drop a pane's last `%output` when
    /// the process exits right after writing, which made this test flaky. The
    /// other live tests end in a sleep for the same reason.
    const STILL_THERE_AFTERWARDS: &str = "read -r said; printf 'said %s\\n' \"$said\"; sleep 120";

    /// Wait for a grid to say something, or give up and show what it did say.
    fn until(grid: &Grid, wanted: &str) -> String {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let seen = shown(grid);
            if seen.contains(wanted) {
                return seen;
            }
            assert!(
                Instant::now() < deadline,
                "gave up waiting for {wanted:?}; the grid says:\n{seen}"
            );
            thread::sleep(Duration::from_millis(25));
        }
    }

    #[test]
    fn what_a_spawn_draws_arrives_in_the_grid_that_owns_its_pane() {
        let tmux = PrivateTmux::start("control-output-reaches-the-grid");
        let session = tmux.server.session(SLOT).unwrap();
        let client = Client::attach(&tmux.server, &session, SLOT).unwrap();

        let one = tmux.server.open_window(&session, "one").unwrap();
        let other = tmux.server.open_window(&session, "other").unwrap();
        let drawing = client.watch(&one, SLOT);
        let quiet = client.watch(&other, SLOT);
        tmux.server
            .start(&one, &tmux.recipe("printf 'from the spawn\\n'; sleep 120"))
            .unwrap();

        until(&drawing, "from the spawn");
        assert!(
            shown(&quiet).trim().is_empty(),
            "output reached a grid that did not own the pane: {}",
            shown(&quiet)
        );
    }

    #[test]
    fn a_spawn_that_started_before_the_app_looked_is_the_case_that_is_avoided() {
        let tmux = PrivateTmux::start("control-streams-from-attaching");
        let session = tmux.server.session(SLOT).unwrap();
        let client = Client::attach(&tmux.server, &session, SLOT).unwrap();
        let pane = tmux.server.open_window(&session, "spawn").unwrap();

        // Grid first, then the process: reversing these two lines is the
        // silent failure this ordering prevents.
        let grid = client.watch(&pane, SLOT);
        tmux.server
            .start(
                &pane,
                &tmux.recipe("printf 'first thing drawn\\n'; sleep 120"),
            )
            .unwrap();

        until(&grid, "first thing drawn");
    }

    #[test]
    fn a_pane_the_app_has_let_go_of_has_no_grid_left_behind() {
        let tmux = PrivateTmux::start("control-forgets-a-retired-pane");
        let session = tmux.server.session(SLOT).unwrap();
        let client = Client::attach(&tmux.server, &session, SLOT).unwrap();
        let pane = tmux.server.open_window(&session, "spawn").unwrap();
        client.watch(&pane, SLOT);

        client.forget(&pane);

        assert!(
            !client.grids.lock().unwrap().contains_key(&pane),
            "a retired spawn's screen is still held"
        );
    }

    #[test]
    fn typing_reaches_the_spawn() {
        let tmux = PrivateTmux::start("control-typing-reaches-the-spawn");
        let session = tmux.server.session(SLOT).unwrap();
        let client = Client::attach(&tmux.server, &session, SLOT).unwrap();
        let pane = tmux.server.open_window(&session, "spawn").unwrap();
        let grid = client.watch(&pane, SLOT);
        tmux.server
            .start(&pane, &tmux.recipe(STILL_THERE_AFTERWARDS))
            .unwrap();

        client.send(&pane, b"hello\r").unwrap();

        until(&grid, "said hello");
    }

    /// The second line is the load-bearing one: it is written a second after
    /// the other spawn's, which is time spent off screen.
    #[test]
    fn a_spawn_the_slot_is_not_showing_keeps_working_and_keeps_arriving() {
        let tmux = PrivateTmux::start("control-spawns-off-screen-keep-going");
        let session = tmux.server.session(SLOT).unwrap();
        let client = Client::attach(&tmux.server, &session, SLOT).unwrap();

        let shown = tmux.server.open_window(&session, "shown").unwrap();
        let off_screen = tmux.server.open_window(&session, "off-screen").unwrap();
        let in_the_slot = client.watch(&shown, SLOT);
        let nobody_is_looking = client.watch(&off_screen, SLOT);
        tmux.server
            .start(&shown, &tmux.recipe("printf 'in the slot\\n'; sleep 120"))
            .unwrap();
        tmux.server
            .start(
                &off_screen,
                &tmux
                    .recipe("printf 'started\\n'; sleep 1; printf 'and still going\\n'; sleep 120"),
            )
            .unwrap();

        until(&in_the_slot, "in the slot");
        until(&nobody_is_looking, "and still going");

        assert!(
            !tmux.server.panes().unwrap().get(&off_screen).unwrap().dead,
            "the spawn nobody was looking at stopped"
        );
    }

    /// tmux answers a child's terminal queries itself, so the app must never
    /// write a reply: a second answer would arrive as keystrokes nobody typed.
    /// This test pins that down.
    #[test]
    fn a_spawn_that_asks_its_terminal_a_question_is_answered_without_the_app() {
        let tmux = PrivateTmux::start("control-terminal-queries-are-answered");
        let session = tmux.server.session(SLOT).unwrap();
        let client = Client::attach(&tmux.server, &session, SLOT).unwrap();
        let pane = tmux.server.open_window(&session, "spawn").unwrap();
        let grid = client.watch(&pane, SLOT);
        tmux.server
            .start(&pane, &tmux.recipe("printf '\\033[6n'; sleep 120"))
            .unwrap();

        let seen = until(&grid, "[1;1R");

        assert!(
            !seen.contains("[1;1R[1;1R"),
            "the spawn was answered twice: {seen}"
        );
    }

    /// The windows not being displayed are the interesting ones: a resize that
    /// missed them would leave spawns drawing at the old shape.
    #[test]
    fn resizing_the_slot_resizes_every_spawn_in_it() {
        const SPAWNS: usize = 4;

        let tmux = PrivateTmux::start("control-resize-reaches-the-spawns");
        let session = tmux.server.session(SLOT).unwrap();
        let client = Client::attach(&tmux.server, &session, SLOT).unwrap();
        let panes: Vec<String> = (0..SPAWNS)
            .map(|which| {
                tmux.server
                    .open_window(&session, &format!("spawn-{which}"))
                    .unwrap()
            })
            .collect();

        client
            .resize(Size {
                columns: 100,
                rows: 30,
            })
            .unwrap();

        tmux.until("#{pane_id} #{pane_width}x#{pane_height}", |seen| {
            panes
                .iter()
                .all(|pane| seen.contains(&format!("{pane} 100x30")))
        });
    }

    #[test]
    fn a_session_that_is_not_there_is_a_refusal_rather_than_a_blank_slot() {
        let tmux = PrivateTmux::start("control-refuses-without-a-session");

        let refused = Client::attach(&tmux.server, "no-such-session", SLOT);

        assert!(refused.is_err(), "attaching to nothing was allowed");
    }

    /// Letting go of the client is what quitting the app does to it.
    #[test]
    fn letting_go_of_the_client_leaves_every_spawn_running() {
        let tmux = PrivateTmux::start("control-quitting-kills-nothing");
        let session = tmux.server.session(SLOT).unwrap();
        let client = Client::attach(&tmux.server, &session, SLOT).unwrap();
        let pane = tmux.server.open_window(&session, "spawn").unwrap();
        let grid = client.watch(&pane, SLOT);
        tmux.server
            .start(&pane, &tmux.recipe("printf 'still here\\n'; sleep 120"))
            .unwrap();
        until(&grid, "still here");

        drop(client);

        thread::sleep(Duration::from_millis(250));
        let alive = tmux.server.panes().unwrap();
        assert!(
            !alive.get(&pane).expect("the spawn's pane went too").dead,
            "the spawn stopped when the app let go of the client"
        );
    }

    /// Not a throughput test: it rules out the single reader mixing panes up
    /// or dropping one under concurrent load, not a ceiling.
    #[test]
    fn one_reader_keeps_several_spawns_apart() {
        const SPAWNS: usize = 8;

        let tmux = PrivateTmux::start("control-one-reader-several-spawns");
        let session = tmux.server.session(SLOT).unwrap();
        let client = Client::attach(&tmux.server, &session, SLOT).unwrap();

        let grids: Vec<Grid> = (0..SPAWNS)
            .map(|which| {
                let pane = tmux
                    .server
                    .open_window(&session, &format!("s{which}"))
                    .unwrap();
                let grid = client.watch(&pane, SLOT);
                tmux.server
                    .start(
                        &pane,
                        &tmux.recipe(&format!(
                            "i=0; while [ $i -lt 40 ]; do printf 'spawn {which} line %s\\n' $i; \
                             i=$((i+1)); done; sleep 120"
                        )),
                    )
                    .unwrap();

                grid
            })
            .collect();

        for (which, grid) in grids.iter().enumerate() {
            let seen = until(grid, &format!("spawn {which} line 39"));
            for other in 0..SPAWNS {
                assert!(
                    other == which || !seen.contains(&format!("spawn {other} ")),
                    "one spawn's output landed in another's grid:\n{seen}"
                );
            }
        }
    }

    #[test]
    fn a_client_does_not_inherit_the_session_its_user_is_sitting_in() {
        let mut client = CommandBuilder::new("tmux");
        client.env("TMUX", "/tmp/tmux-1000/default,4242,0");
        client.env("TMUX_PANE", "%7");

        forget_the_users_session(&mut client);

        assert!(client.get_env("TMUX").is_none());
        assert!(client.get_env("TMUX_PANE").is_none());
    }
}
