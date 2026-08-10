//! The one client every spawn's output arrives down.
//!
//! tmux in control mode is not a terminal: it is a stream of notifications, one
//! of which — `%output` — carries the bytes a pane produced, tagged with the
//! pane that produced them. **One client carries every pane in the session**,
//! including the windows nobody is looking at, which is what makes a single
//! reader serve twenty spawns rather than twenty readers serving one each.
//!
//! Two things about it were learned the hard way, and both are load-bearing:
//!
//! - **A control client needs a terminal of its own.** On piped stdio tmux
//!   refuses outright — `tcgetattr failed: Inappropriate ioctl for device` — so
//!   the client runs inside a pty the app opens for it. One pty for the whole
//!   app; the children's belong to tmux.
//! - **It streams only what is produced while it is attached.** A pane that drew
//!   itself before anyone was listening stays blank for ever, with no catching
//!   up short of priming from `capture-pane`. The app never has to: it attaches
//!   before it starts anything, and a spawn's window is opened holding nothing
//!   but a placeholder so that its grid is already listening when the harness
//!   replaces it.
//!
//! Everything else tmux is asked — make a window, start something in it, is it
//! still alive — goes the other way, as an ordinary command, because control
//! mode cannot report a process dying. Only two things travel back up this
//! client: keystrokes, and the size of the slot.

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
///
/// Generous, because it costs nothing when it is not needed: tmux answers an
/// attach immediately or not at all, so this is only ever spent on the way to a
/// refusal.
const ATTACHING: Duration = Duration::from_secs(5);

/// The grid behind one pane, shared between the reader and whatever draws it.
pub type Grid = Arc<Mutex<Screen>>;

/// Which grid each pane's output belongs to.
type Grids = Arc<Mutex<HashMap<String, Grid>>>;

/// An attached control-mode client.
///
/// Dropping it hangs the client up — and takes nothing with it. The session and
/// every spawn in it belong to the tmux server, which is a different process
/// and outlives this one.
pub struct Client {
    /// The pty the client runs in, kept because closing it is what ends the
    /// client.
    terminal: Box<dyn MasterPty + Send>,
    /// What the app says back: keystrokes, and the size of the slot.
    saying: Arc<Mutex<Box<dyn Write + Send>>>,
    /// The grids output is routed into.
    grids: Grids,
    /// Why the reader stopped, once it has.
    ended: Ended,
}

/// Why the app stopped being told what the spawns are drawing, once it has.
///
/// A control client that goes takes every slot with it, and takes them
/// *quietly*: the grids stay exactly as they were, which on screen is a session
/// that has stopped moving rather than anything that looks like a failure. The
/// reader therefore leaves its reason behind, and the app looks for one every
/// frame.
type Ended = Arc<OnceLock<String>>;

impl Client {
    /// Attach to a session, and start reading everything in it.
    ///
    /// The size is the slot's, because the slot is what every spawn draws into:
    /// a control client's size is the size of every window in the session it is
    /// attached to, so this one number shapes them all.
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
    /// Everything the pane produces from now on lands in it. Do this **before**
    /// starting anything in that pane: output produced before there is a grid to
    /// route it into is not held anywhere, and cannot be asked for again.
    pub fn watch(&self, pane: &str, size: Size) -> Grid {
        let grid: Grid = Arc::new(Mutex::new(Screen::new(size)));
        self.grids
            .lock()
            .expect(POISONED)
            .insert(pane.to_string(), Arc::clone(&grid));

        grid
    }

    /// Type at a spawn.
    ///
    /// Bytes as hex, which is the one encoding that survives arbitrary
    /// keystrokes without anything having to be quoted for tmux's own parser.
    pub fn send(&self, pane: &str, bytes: &[u8]) -> Result<()> {
        if bytes.is_empty() {
            return Ok(());
        }

        self.say(&send_keys(pane, bytes))
    }

    /// Say how big the slot is now.
    ///
    /// One call for every spawn there is: the windows in a session follow the
    /// size of the client attached to it, so this is what makes the app's own
    /// window being resized reach twenty children at once.
    pub fn resize(&self, slot: Size) -> Result<()> {
        self.terminal.resize(sized(slot)).map_err(|trouble| {
            Error::new(format!(
                "the control-mode client would not be resized: {trouble}"
            ))
        })?;

        self.say(&format!("refresh-client -C {}x{}", slot.columns, slot.rows))
    }

    /// Refuse once there is nothing on the other end any more.
    ///
    /// Asked every frame rather than waited on: what a hung-up client leaves
    /// behind is a screenful of grids that have simply stopped changing, and a
    /// session that looks like it is thinking is the one thing the app must
    /// never show when it is not.
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

/// What a lock says when the thread holding it died mid-write.
///
/// Nothing here can leave a grid or the client half-written: the reader applies
/// whole chunks and the writer whole lines. A poisoned lock therefore means a
/// panic somewhere else entirely, and carrying on with a screen nobody can
/// explain is worse than stopping.
///
/// Said once, here, because the reader and whatever is drawing take the same
/// locks and two wordings of it would read as two different faults.
pub const POISONED: &str = "another thread panicked while holding a terminal";

/// Read the client until it hangs up, routing output to the grid that owns it.
///
/// Bytes rather than text. What a spawn draws is not obliged to be valid UTF-8
/// — and even when it is, tmux chops the stream wherever the child's writes
/// happened to land, so half a character at the end of one line is ordinary.
/// Reading this as text would turn that into an error and lose the rest of the
/// spawn's life.
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
        // The map is let go of before the grid is taken, so a slow frame being
        // drawn from one spawn's grid never holds up another spawn's output.
        let grid = grids.lock().expect(POISONED).get(&output.pane).cloned();
        if let Some(grid) = grid {
            grid.lock().expect(POISONED).apply(&output.bytes);
        }
    }
}

/// Leave behind why the reader stopped, for the app to find.
///
/// The first reason wins, because a client that has already gone cannot go
/// again — and the first one is the one that explains the others.
fn stopped(ended: &Ended, why: String) {
    let _ = ended.set(why);
}

/// Keep a client from inheriting the session its user is sitting in.
///
/// tmux reads these as an attempt to nest a session inside itself and refuses.
/// The app's own server is not the one they are sitting in — but tmux has no way
/// to know that, and this is where saying so costs nothing.
fn forget_the_users_session(client: &mut CommandBuilder) {
    client.env_remove("TMUX");
    client.env_remove("TMUX_PANE");
}

/// Whether there is a client to talk to.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Attaching {
    /// tmux took the client, and everything in the session now streams.
    Attached,
    /// It did not, and this is what it said about why.
    Refused(String),
}

/// Reading the first thing tmux says, which is whether it attached at all.
///
/// A control client that will not attach is not quiet — it says so and hangs
/// up. But it says so *inside* the reply to the attach, which looks exactly like
/// the reply to a command that worked until the last line of it: `%begin`,
/// whatever there is to say, and then either `%end` or `%error`. Deciding on the
/// first line alone reads a refusal as a greeting, and the app carries on to a
/// slot that will stay blank for ever.
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

        // The `%begin` is wrapped in an escape sequence of its own on the very
        // first line, so these are looked for anywhere on the line rather than
        // at the start of it.
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
    /// Read one line of the client, if it was output at all.
    ///
    /// Everything else tmux says — that a window was added, that a command
    /// finished, that the layout changed — is not output and is skipped. The app
    /// asks its questions as commands and reads its answers there.
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

/// A line without the terminator the pty put on it.
///
/// The client writes into a terminal, and a terminal turns every newline into a
/// carriage return and a newline. Left on, that carriage return would reach a
/// grid as something the spawn drew.
fn trimmed(line: &[u8]) -> &[u8] {
    let mut end = line.len();
    while end > 0 && (line[end - 1] == b'\n' || line[end - 1] == b'\r') {
        end -= 1;
    }

    &line[..end]
}

/// Turn what tmux escaped back into what the spawn wrote.
///
/// Control mode is a line protocol, so anything that could end a line is sent
/// as three octal digits behind a backslash — and so is the backslash itself,
/// which is what makes this unambiguous. Everything else, UTF-8 included, is
/// passed through untouched.
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

/// One `\ooo` escape, if that is what these four bytes are.
///
/// Three octal *digits* and nothing else — a sign or a space would be read as a
/// number by the usual parse, and turn a backslash the spawn drew into a byte it
/// never wrote.
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

    // The rest drives a real tmux and a real pty, on a socket of this test's
    // own. There is no fake for either: what is being tested is exactly the
    // part a fake would have to pretend about.

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
    /// **The `sleep` is load-bearing, and finding out why cost an afternoon of
    /// a flaky test.** A pane whose process exits the instant it has written is
    /// a pane whose last write tmux may never send a control client: the bytes
    /// reach the pane's own screen — `capture-pane` shows them — but the
    /// `%output` notification carrying them is dropped along with the pane's
    /// closing file descriptor. It is a race, so it is intermittent, and under
    /// load it was hit about half the time.
    ///
    /// Nothing about the app depends on that being fixed: a harness is a
    /// long-running program, and the ladder in [`crate::snapshot`] is what
    /// reports one that has stopped. What did depend on it was this test, which
    /// was written to prove that typing arrives and was failing for a reason
    /// that had nothing to do with typing. The two live tests either side of it
    /// end in a sleep for the same reason.
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

        // What the app does, in the order it does it: a grid first, and only
        // then something to draw into it. Reversing these two lines is the
        // failure this ordering exists to prevent, and it is silent.
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

    /// The sharpest risk the design names, and the reason it is not one.
    ///
    /// A program asks its terminal where the cursor is and waits for an answer.
    /// The app's own emulator has no way to reply — and never needs one: tmux is
    /// the terminal the spawn is really talking to, and it answers before the
    /// bytes are passed on. Here the spawn asks and nothing else does anything;
    /// the answer arrives on the pane's own input, and the pane's terminal
    /// echoes what it is sent, which is how it comes to be on screen at all. An
    /// answer of the app's own would arrive after this one, as keystrokes
    /// nobody typed.
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

    #[test]
    fn resizing_the_slot_resizes_every_spawn_in_it() {
        let tmux = PrivateTmux::start("control-resize-reaches-the-spawns");
        let session = tmux.server.session(SLOT).unwrap();
        let client = Client::attach(&tmux.server, &session, SLOT).unwrap();
        let pane = tmux.server.open_window(&session, "spawn").unwrap();

        client
            .resize(Size {
                columns: 100,
                rows: 30,
            })
            .unwrap();

        tmux.until("#{pane_id} #{pane_width}x#{pane_height}", |seen| {
            seen.contains(&format!("{pane} 100x30"))
        });
    }

    #[test]
    fn a_session_that_is_not_there_is_a_refusal_rather_than_a_blank_slot() {
        let tmux = PrivateTmux::start("control-refuses-without-a-session");

        let refused = Client::attach(&tmux.server, "no-such-session", SLOT);

        assert!(refused.is_err(), "attaching to nothing was allowed");
    }

    /// The promise the whole mechanism is for.
    ///
    /// Letting go of the client is what quitting the app does to it, so this is
    /// that moment: the reader stops, the pty closes, and the session and every
    /// spawn in it carry on because they were never the app's to begin with.
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

    /// One reader, several spawns, all drawing at once.
    ///
    /// The design names the single reader as the sharpest unknown it introduces
    /// and asks for it to be measured rather than argued about. This is the
    /// cheap end of that: every spawn draws something only it could have drawn,
    /// and every grid has to end up with its own. **It is not twenty fullscreen
    /// agents** — what it rules out is the reader mixing panes up or dropping
    /// one under concurrent load, not a throughput ceiling.
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
