//! Twenty spawns at once: a measurement rig, not a behaviour test. The numbers
//! it produced live in `docs/evidence/scale-at-twenty.md`. `#[ignore]`d — run
//! it by hand:
//!
//! ```text
//! cargo test --release --test twenty_spawns -- --ignored --nocapture --test-threads=1
//! ```
//!
//! No real harness runs: the panes hold a stand-in doing the two things being
//! measured — repainting a whole screen on the alternate buffer and keeping a
//! session record where the harness keeps one. Everything (HOME, config,
//! worktrees, the tmux socket) is private to the run. This file may name the
//! harness: the no-Claude-Code rule covers `src/`, and this rig is the world
//! the app runs in, not part of it.

use std::fs;
use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use tempfile::TempDir;

/// The terminal the app is given; large, so twenty rows sit beside a session.
const COLUMNS: u16 = 200;
const ROWS: u16 = 50;

/// How long anything waited for is waited for.
const PATIENCE: Duration = Duration::from_secs(30);

/// The app's own supervisor interval, repeated here because this file drives
/// the binary rather than linking to it.
const TICK: Duration = Duration::from_millis(200);

/// The app's unaccounted-for grace period plus a little, so a screen taken
/// after this has all three statuses on it.
const GRACE: Duration = Duration::from_secs(12);

/// Several repositories, because "spawns on different repositories" is what
/// the tranche promises.
const REPOSITORIES: [&str; 4] = ["harness-launcher", "acme-api", "dotfiles", "infra"];

/// Twenty pieces of work, five per repository, named the way a person would —
/// the list's readability is one of the things being looked at.
const WORK: [&str; 20] = [
    "fix worktree cleanup",
    "add retry logic",
    "status ladder grace period",
    "spawn form choices",
    "control mode backpressure",
    "rate limit headers",
    "drop legacy auth",
    "openapi drift check",
    "pagination cursors",
    "idempotency keys",
    "tidy the shell prompt",
    "neovim lsp config",
    "ssh agent forwarding",
    "font fallback for emoji",
    "prune stale symlinks",
    "terraform state locking",
    "rotate the deploy keys",
    "alert on disk pressure",
    "cheaper log retention",
    "blue green cutover",
];

/// The spawns that report themselves stopped, and the one that writes no
/// record — so the list has all three statuses on it; carried by the model
/// rather than the description.
const STOPPED: [usize; 3] = [2, 9, 16];
const SILENT: usize = 13;

// --- The rig ---

/// Everywhere the run keeps something, and everything it made.
struct Rig {
    root: TempDir,
}

impl Rig {
    fn new() -> Self {
        let rig = Self {
            root: tempfile::tempdir().unwrap(),
        };
        for directory in ["bin", "home", "config", "data", "sockets", "repositories"] {
            fs::create_dir_all(rig.at(directory)).unwrap();
        }
        write_program(&rig.at("bin").join("claude"), &stand_in());
        write_program(&rig.at("bin").join("git"), SLOW_GIT);

        rig
    }

    fn at(&self, what: &str) -> PathBuf {
        self.root.path().join(what)
    }

    /// A repository with an `origin` whose `HEAD` resolves, which is the least
    /// the app will start a spawn against.
    fn repository(&self, name: &str) -> PathBuf {
        let origin = self.at("repositories").join(format!("{name}.git"));
        let clone = self.at("repositories").join(name);
        git(&["init", "--bare", "-b", "main", text(&origin)]);
        git(&["init", "-b", "main", text(&clone)]);
        git(&["-C", text(&clone), "commit", "--allow-empty", "-m", "first"]);
        git(&["-C", text(&clone), "remote", "add", "origin", text(&origin)]);
        git(&["-C", text(&clone), "push", "-u", "origin", "main"]);
        git(&["-C", text(&clone), "remote", "set-head", "origin", "--auto"]);

        clone
    }

    /// The environment every process this rig starts is given.
    fn environment(&self) -> Vec<(String, String)> {
        let path = format!(
            "{}:{}",
            text(&self.at("bin")),
            std::env::var("PATH").unwrap_or_default()
        );

        vec![
            ("PATH".to_string(), path),
            ("HOME".to_string(), text(&self.at("home")).to_string()),
            (
                "XDG_DATA_HOME".to_string(),
                text(&self.at("data")).to_string(),
            ),
            (
                "CLAUDE_CONFIG_DIR".to_string(),
                text(&self.at("config")).to_string(),
            ),
            (
                "TMUX_TMPDIR".to_string(),
                text(&self.at("sockets")).to_string(),
            ),
            ("TERM".to_string(), "xterm-256color".to_string()),
        ]
    }

    /// Ask the rig's own tmux server something — via the rig's socket
    /// directory, so it is never the user's server that answers.
    fn tmux(&self, arguments: &[&str]) -> String {
        let mut command = Command::new("tmux");
        command.args(["-L", "harness-launcher"]).args(arguments);
        for (name, value) in self.environment() {
            command.env(name, value);
        }
        let outcome = command.output().unwrap();

        String::from_utf8_lossy(&outcome.stdout)
            .trim_end()
            .to_string()
    }
}

impl Drop for Rig {
    /// Quitting the app kills nothing, so the rig is what stops the stand-ins.
    fn drop(&mut self) {
        self.tmux(&["kill-server"]);
    }
}

/// A stand-in for the harness, doing its two observable jobs: it repaints a
/// whole screen on the alternate buffer several times a second, and it writes
/// a session record keyed by its pane's pid. The model chooses its status:
/// `haiku` stopped, `sonnet` no record at all, anything else busy.
/// `__RECORD__` is filled in by [`stand_in`].
const STAND_IN: &str = r#"#!/bin/bash
model=""; name=""; work=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    --model) model="$2"; shift 2 ;;
    --effort) shift 2 ;;
    -n) name="$2"; shift 2 ;;
    *) work="$1"; shift ;;
  esac
done

case "$model" in
  haiku) status=idle ;;
  sonnet) status="" ;;
  *) status=busy ;;
esac

record() {
  [ -z "$status" ] && return 0
  mkdir -p "$CLAUDE_CONFIG_DIR/sessions"
  printf '__RECORD__\n' "$status" "$$" > "$CLAUDE_CONFIG_DIR/sessions/$$.json"
}
record

printf '\033[?1049h\033[?25l'
trap 'printf "\033[?1049l"; exit 0' TERM INT HUP

filler="the quick brown fox jumps over the lazy dog and keeps on running for a while"
turn=0
typed=""
while :; do
  turn=$((turn + 1))
  printf '\033[H\033[2J'
  printf 'at=%s turn=%d\r\n' "$EPOCHREALTIME" "$turn"
  printf '> %s\r\n\r\n' "$work"
  line=0
  while [ "$line" -lt 40 ]; do
    line=$((line + 1))
    printf '  <<%s>> %02d %s\r\n' "$name" "$line" "$filler"
  done
  printf '\r\ntyped: %s\r\n' "$typed"
  [ $((turn % 10)) -eq 0 ] && record
  if [ "$status" = idle ]; then
    IFS= read -r -s -N1 key && typed="$typed$key"
  else
    IFS= read -r -s -N1 -t 0.2 key && typed="$typed$key"
  fi
done
"#;

/// The captured session record — see `captured/README.md`.
const RECORD: &str = include_str!("../captured/session-record.json");

/// The stand-in's record, filled in from the capture rather than written from
/// memory (see `captured/README.md`), with the same two substitutions the
/// app's unit tests make: a `status` field and the stand-in's own pid.
fn stand_in() -> String {
    let rest = RECORD
        .trim()
        .strip_prefix('{')
        .expect("the captured record is an object");
    let record = format!("{{\"status\":\"%s\",{rest}").replace("\"pid\":531", "\"pid\":%d");

    STAND_IN.replace("__RECORD__", &record)
}

/// `git`, with one worktree made slowly on purpose: worktree creation is on a
/// thread of its own precisely because it takes seconds, and an instant `git`
/// would leave that untested. The real `git` is found by dropping `PATH`'s
/// first entry, which is always the rig's own bin.
const SLOW_GIT: &str = r#"#!/bin/bash
for argument in "$@"; do
  case "$argument" in
    *takes-its-time*) sleep 6; break ;;
  esac
done
PATH="${PATH#*:}" exec git "$@"
"#;

fn write_program(path: &Path, contents: &str) {
    fs::write(path, contents).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

/// `git` for the rig's own setup; this process's `PATH` has no rig on it, so
/// it never goes through the slow one.
fn git(arguments: &[&str]) {
    let outcome = Command::new("git")
        .args(arguments)
        .env("GIT_AUTHOR_NAME", "rig")
        .env("GIT_AUTHOR_EMAIL", "rig@example.com")
        .env("GIT_COMMITTER_NAME", "rig")
        .env("GIT_COMMITTER_EMAIL", "rig@example.com")
        .output()
        .unwrap();
    assert!(
        outcome.status.success(),
        "git {arguments:?} failed: {}",
        String::from_utf8_lossy(&outcome.stderr)
    );
}

fn text(path: &Path) -> &str {
    path.to_str().unwrap()
}

// --- The app, on a terminal of the rig's own ---

/// The app's terminal, held as a live emulator rather than a pile of bytes:
/// the app writes only the cells that changed, so the raw stream carries
/// fragments nothing could be searched for.
struct Terminal {
    /// What is on the screen now.
    parser: vt100::Parser,
    /// How much has arrived, which is how a wait tells new output from old.
    written: usize,
}

impl Default for Terminal {
    fn default() -> Self {
        Self {
            parser: vt100::Parser::new(ROWS, COLUMNS, 0),
            written: 0,
        }
    }
}

impl Terminal {
    fn screen(&self) -> String {
        self.parser.screen().contents()
    }
}

/// The app, running on a terminal this rig opened.
struct App {
    terminal: Arc<Mutex<Terminal>>,
    writer: Box<dyn Write + Send>,
    child: Box<dyn portable_pty::Child + Send + Sync>,
    pid: u32,
}

impl Drop for App {
    /// Killed by the rig, since quitting the app kills nothing. Both the app
    /// and the tracer, which under `strace` are two processes.
    fn drop(&mut self) {
        let _ = Command::new("kill")
            .args(["-9", &self.pid.to_string()])
            .output();
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl App {
    /// Start it, with everything it is to spawn on the command line. `under`
    /// is a file for `strace` to write to, used only by the tick-cost run
    /// because tracing is not free.
    fn start(rig: &Rig, arguments: &[String], under: Option<&Path>) -> Self {
        let pty = native_pty_system()
            .openpty(PtySize {
                rows: ROWS,
                cols: COLUMNS,
                pixel_width: 0,
                pixel_height: 0,
            })
            .unwrap();

        let mut command = match under {
            Some(log) => {
                let mut strace = CommandBuilder::new("strace");
                strace.args([
                    "-f",
                    "--seccomp-bpf",
                    "-qq",
                    "-ttt",
                    "-e",
                    "trace=execve,statx,newfstatat,openat",
                    "-o",
                    text(log),
                    env!("CARGO_BIN_EXE_harness-launcher"),
                ]);
                strace
            }
            None => CommandBuilder::new(env!("CARGO_BIN_EXE_harness-launcher")),
        };
        command.args(arguments);
        command.env_remove("TMUX");
        command.env_remove("TMUX_PANE");
        for (name, value) in rig.environment() {
            command.env(name, value);
        }

        let child = pty.slave.spawn_command(command).unwrap();
        drop(pty.slave);
        // Under the tracer the app is the tracer's child, and the app's own
        // process is the one measured.
        let pid = match under {
            None => child.process_id().unwrap(),
            Some(_) => traced_by(child.process_id().unwrap()),
        };

        let terminal = Arc::new(Mutex::new(Terminal::default()));
        let mut reader = pty.master.try_clone_reader().unwrap();
        let drawing = Arc::clone(&terminal);
        thread::spawn(move || {
            let mut buffer = vec![0u8; 65536];
            while let Ok(read) = reader.read(&mut buffer) {
                if read == 0 {
                    break;
                }
                let mut terminal = drawing.lock().unwrap();
                terminal.parser.process(&buffer[..read]);
                terminal.written += read;
            }
        });

        Self {
            terminal,
            writer: pty.master.take_writer().unwrap(),
            child,
            pid,
        }
    }

    /// The screen as it stands.
    fn screen(&self) -> String {
        self.terminal.lock().unwrap().screen()
    }

    /// How many bytes it has written, which is how much drawing it did.
    fn written(&self) -> usize {
        self.terminal.lock().unwrap().written
    }

    /// Wait for the screen to say something, and return when it did (polled
    /// every millisecond).
    fn wait_for(&mut self, wanted: &str) -> Instant {
        let deadline = Instant::now() + PATIENCE;
        loop {
            if self.screen().contains(wanted) {
                return Instant::now();
            }
            assert!(
                Instant::now() < deadline,
                "the app never drew {wanted:?}{}\n{}",
                self.still_running(),
                self.screen()
            );
            thread::sleep(Duration::from_millis(1));
        }
    }

    /// A stuck app and a dead app are different faults; every complaint says
    /// which.
    fn still_running(&mut self) -> String {
        match self.child.try_wait() {
            Ok(Some(status)) => format!(" — and the app has stopped: {status:?}"),
            Ok(None) => " — the app is still running".to_string(),
            Err(trouble) => format!(" — and the app cannot be asked how it is: {trouble}"),
        }
    }

    /// Type at it.
    fn types(&mut self, bytes: &[u8]) -> Instant {
        self.writer.write_all(bytes).unwrap();
        self.writer.flush().unwrap();

        Instant::now()
    }

    /// Which spawn the slot is showing: each stand-in writes its name between
    /// guillemets, and nothing else on the screen does.
    fn showing(&self) -> Option<String> {
        let screen = self.screen();
        let name = screen.split("<<").nth(1)?.split(">>").next()?;

        Some(name.to_string())
    }

    /// Move the selection, and time until the slot shows another spawn. Where
    /// it lands is deliberately not predicted: the list re-sorts, and a
    /// predicted target could already be on screen — a switch of no time at
    /// all.
    fn switch(&mut self, key: &[u8]) -> Duration {
        let before = self.showing();
        let sent = self.types(key);
        let deadline = sent + PATIENCE;
        loop {
            let now = self.showing();
            if now.is_some() && now != before {
                return sent.elapsed();
            }
            assert!(
                Instant::now() < deadline,
                "the selection did not move off {before:?}{}\n{}",
                self.still_running(),
                self.screen()
            );
            thread::sleep(Duration::from_millis(1));
        }
    }

    /// How far behind what a spawn drew the screen is running, via the
    /// stand-in's timestamp — the whole path, child's write to rig emulator.
    /// One pane, not twenty: the lag of the spawn being looked at while
    /// nineteen others load the reader; the evidence document says so.
    fn lag(&self) -> Option<Duration> {
        let screen = self.screen();
        let stamped = screen.split("at=").nth(1)?;
        let drawn: f64 = stamped.split_whitespace().next()?.parse().ok()?;

        Duration::try_from_secs_f64(epoch_now() - drawn).ok()
    }

    /// How much memory it is holding.
    fn resident(&self) -> u64 {
        resident(self.pid)
    }
}

/// The resident memory of a process, in kilobytes.
fn resident(pid: u32) -> u64 {
    let status = fs::read_to_string(format!("/proc/{pid}/status")).unwrap_or_default();

    status
        .lines()
        .find_map(|line| line.strip_prefix("VmRSS:"))
        .and_then(|line| line.split_whitespace().next()?.parse().ok())
        .unwrap_or_default()
}

/// The keys the app answers to.
const UP: &[u8] = b"\x1b[17~";
const DOWN: &[u8] = b"\x1b[18~";
const COMPOSE: &[u8] = b"\x1bOQ";
const START: &[u8] = b"\x1b[15~";
const QUIT: &[u8] = b"\x1b[21~";
const TAB: &[u8] = b"\t";

// --- What was measured ---

/// A set of timings, said the way a report wants them.
fn spread(name: &str, timings: &[Duration]) -> String {
    let mut sorted: Vec<f64> = timings
        .iter()
        .map(|timing| timing.as_secs_f64() * 1000.0)
        .collect();
    sorted.sort_by(f64::total_cmp);
    // An empty sample set is a result to print, not a crash.
    let Some(worst) = sorted.last().copied() else {
        return format!("{name}: nothing was sampled");
    };
    let at = |fraction: usize| sorted[(sorted.len() - 1) * fraction / 100];

    format!(
        "{name}: {} samples, median {:.1} ms, mean {:.1} ms, p95 {:.1} ms, worst {:.1} ms",
        sorted.len(),
        at(50),
        sorted.iter().sum::<f64>() / how_many(sorted.len()),
        at(95),
        worst,
    )
}

#[test]
fn a_measurement_that_collected_no_samples_is_reported_rather_than_fatal() {
    assert_eq!(
        spread("how far behind what a spawn drew the screen ran", &[]),
        "how far behind what a spawn drew the screen ran: nothing was sampled"
    );
}

#[test]
fn a_single_sample_is_its_own_median_and_its_own_worst() {
    let said = spread("a lag", &[Duration::from_millis(7)]);

    assert!(said.contains("1 samples"), "{said}");
    assert!(said.contains("median 7.0 ms"), "{said}");
    assert!(said.contains("worst 7.0 ms"), "{said}");
}

/// A count, as something to divide by.
fn how_many(counted: usize) -> f64 {
    f64::from(u32::try_from(counted).unwrap_or(u32::MAX))
}

/// Now, as the stand-ins write it.
fn epoch_now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs_f64()
}

/// One test rather than six: each measurement is about a machine carrying the
/// other nineteen spawns while it is taken. Each step reports the moment it
/// knows, so a later failure keeps earlier results.
#[test]
#[ignore = "a measurement rig: it takes a minute and wants the machine to itself"]
fn twenty_spawns_at_once() {
    let rig = Rig::new();
    let repositories: Vec<PathBuf> = REPOSITORIES
        .iter()
        .map(|name| rig.repository(name))
        .collect();

    let started = Instant::now();
    let mut app = App::start(&rig, &asked_for(&repositories), None);

    let names = all_of_them_live(&rig, &mut app, started);
    what_is_held(&rig, &mut app);
    how_far_behind(&mut app);
    switching_between_them(&mut app, names.len());
    a_creation_that_takes_its_time(&mut app, &repositories);
    what_it_leaves_behind(&mut app);
}

/// Twenty spawns live over four repositories, and what the list says once
/// every status has settled.
fn all_of_them_live(rig: &Rig, app: &mut App, started: Instant) -> Vec<String> {
    for work in WORK {
        app.wait_for(&work.replace(' ', "-"));
    }
    said(&format!(
        "twenty spawns over {} repositories were created, started and drawn in {:.1} s",
        REPOSITORIES.len(),
        started.elapsed().as_secs_f64()
    ));

    let windows = rig.tmux(&[
        "list-panes",
        "-s",
        "-t",
        "spawns",
        "-F",
        "#{window_name} #{pane_dead}",
    ]);
    let live = windows
        .lines()
        .filter(|line| !line.starts_with("holding ") && line.ends_with(" 0"))
        .count();
    assert_eq!(live, WORK.len(), "not twenty live panes:\n{windows}");
    said(&format!(
        "tmux holds {} windows in one session: the holding window and {live} live spawns",
        windows.lines().count()
    ));

    // Past the grace period, so the unreadable spawn has settled into
    // `unknown` and all three statuses are on the list at once.
    thread::sleep(GRACE);
    let settled = app.screen();
    let names = names_on(&settled);
    assert_eq!(
        names.len(),
        WORK.len(),
        "the list does not name twenty spawns:\n{settled}"
    );
    said(&format!("the list, at twenty:\n\n{}", listed(&settled)));

    names
}

/// What the two processes hold, and whether thirty seconds of twenty spawns
/// redrawing moves it.
fn what_is_held(rig: &Rig, app: &mut App) {
    let server: u32 = rig
        .tmux(&["display-message", "-p", "#{pid}"])
        .parse()
        .unwrap();
    let (first_server, first_app) = (resident(server), app.resident());
    let drawn = app.written();

    thread::sleep(Duration::from_secs(30));

    said(&format!(
        "the tmux server was holding {} MB at twenty panes, and {} MB thirty seconds later\n\
         the app was holding {} MB, and {} MB thirty seconds later\n\
         the app drew {} KB in those thirty seconds",
        first_server / 1024,
        resident(server) / 1024,
        first_app / 1024,
        app.resident() / 1024,
        (app.written() - drawn) / 1024,
    ));
}

/// How far behind what a spawn drew the screen runs, while twenty of them draw.
fn how_far_behind(app: &mut App) {
    let mut lags = Vec::new();
    for _ in 0..50 {
        if let Some(lag) = app.lag() {
            lags.push(lag);
        }
        thread::sleep(Duration::from_millis(100));
    }

    said(&spread(
        "how far behind what a spawn drew the screen ran",
        &lags,
    ));
}

/// Switching, mid-turn: nineteen moves each way, since the selection stops at
/// the ends rather than wrapping; walked to the top first because the opening
/// selection has since moved into the middle of its group.
fn switching_between_them(app: &mut App, spawns: usize) {
    for _ in 0..spawns {
        app.types(UP);
    }
    thread::sleep(Duration::from_secs(1));

    let mut switches = Vec::new();
    for _ in 1..spawns {
        switches.push(app.switch(DOWN));
    }
    for _ in 1..spawns {
        switches.push(app.switch(UP));
    }

    said(&spread(
        "switching between spawns, mid-turn, right down the list and back",
        &switches,
    ));
}

/// A creation made to take six seconds, and whether anything else waits for it.
fn a_creation_that_takes_its_time(app: &mut App, repositories: &[PathBuf]) {
    let slow = "creation that takes its time";
    app.types(COMPOSE);
    app.types(text(&repositories[0]).as_bytes());
    app.types(TAB);
    app.types(slow.as_bytes());
    let asking = Instant::now();
    app.types(START);

    // The app reads one keystroke a frame, so the form is waited for. The
    // selection then walks between two spawns below the draft — never back
    // onto it, since its slot holds a form rather than a session.
    app.wait_for("NEW SPAWN");
    let mut during = Vec::new();
    let mut drawing = 0;
    app.switch(DOWN);
    app.switch(DOWN);
    let mut up = true;
    while asking.elapsed() < Duration::from_secs(5) {
        let before = app.written();
        during.push(app.switch(if up { UP } else { DOWN }));
        drawing += app.written() - before;
        up = !up;
        thread::sleep(Duration::from_millis(100));
    }
    said(&format!(
        "{}\nand it drew {} KB while the creation ran",
        spread("switching while a six-second creation runs", &during),
        drawing / 1024
    ));

    app.wait_for(&slow.replace(' ', "-"));
    said(&format!(
        "the twenty-first spawn was on the list {:.1} s after it was asked for",
        asking.elapsed().as_secs_f64()
    ));
}

/// What it says it is leaving behind, on the way out.
fn what_it_leaves_behind(app: &mut App) {
    app.types(QUIT);
    app.wait_for("harness-launcher: ");

    let leaving = app
        .screen()
        .lines()
        .find(|line| line.contains("harness-launcher: "))
        .unwrap_or_default()
        .trim()
        .to_string();
    said(&format!("on the way out it said: {leaving}"));
}

/// The command line that asks for twenty spawns.
fn asked_for(repositories: &[PathBuf]) -> Vec<String> {
    let mut arguments: Vec<String> = Vec::new();
    for (at, work) in WORK.iter().enumerate() {
        if at > 0 {
            arguments.push("--and".to_string());
        }
        arguments.push(text(&repositories[at % REPOSITORIES.len()]).to_string());
        arguments.push((*work).to_string());
        if STOPPED.contains(&at) {
            arguments.extend(["--model".to_string(), "haiku".to_string()]);
        } else if at == SILENT {
            arguments.extend(["--model".to_string(), "sonnet".to_string()]);
        }
    }

    arguments
}

/// Report the moment it is known, so a later failure keeps earlier results.
fn said(what: &str) {
    println!("\n--- {what}");
}

/// The lines of a screen that are the list, which is its left third.
fn listed(screen: &str) -> String {
    screen
        .lines()
        .map(|line| {
            line.chars()
                .take(66)
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect::<Vec<String>>()
        .join("\n")
}

/// Every spawn the list names, in the order it draws them.
fn names_on(screen: &str) -> Vec<String> {
    let mut found = Vec::new();
    for line in screen.lines() {
        let list: String = line.chars().take(66).collect();
        for work in WORK {
            let slug = work.replace(' ', "-");
            let Some(at) = list.find(&slug) else {
                continue;
            };
            let name: String = list[at..]
                .chars()
                .take_while(|character| character.is_ascii_alphanumeric() || *character == '-')
                .collect();
            if !found.contains(&name) {
                found.push(name);
            }
        }
    }

    found
}

/// The design's tick-cost claims — one `list-panes` per tick, a stat per live
/// spawn only when it moved, `ps` only as a tie-breaker — counted under
/// `strace`. The numbers are ratios, not speeds: tracing slows everything.
/// Run it with `--test-threads=1` or it measures contention.
#[test]
#[ignore = "a measurement rig: it takes a minute and wants the machine to itself"]
fn what_a_tick_costs_at_twenty_spawns() {
    let rig = Rig::new();
    let repositories: Vec<PathBuf> = REPOSITORIES
        .iter()
        .map(|name| rig.repository(name))
        .collect();
    let log = rig.at("strace.log");
    let mut app = App::start(&rig, &asked_for(&repositories), Some(&log));

    for work in WORK {
        app.wait_for(&work.replace(' ', "-"));
    }
    // Past the grace period, so the unaccounted-for spawn is running its
    // tie-breaker — the per-spawn cost a tick must not have twenty of.
    thread::sleep(GRACE);

    let from = epoch_now();
    thread::sleep(Duration::from_secs(10));
    let to = epoch_now();

    let traced = fs::read_to_string(&log).unwrap();
    if let Some(keep) = std::env::var_os("KEEP_TRACE") {
        fs::copy(&log, PathBuf::from(keep)).unwrap();
    }

    // Stand-ins are traced too, so what each line is about is read off the
    // line, and every program seen is reported rather than only the expected
    // ones. Attributing by parent (tracing `clone`) slowed the app roughly
    // fifty-fold, which is not a measurement.
    let mut ticks = 0;
    let mut ticked: Vec<f64> = Vec::new();
    let mut ran: Vec<String> = Vec::new();
    let mut stats = 0;
    let mut reads = 0;
    for line in traced.lines() {
        let Some((_, at, call)) = traced_call(line) else {
            continue;
        };
        if at < from || at > to {
            continue;
        }

        if call.starts_with("execve(") {
            // Only successful ones: the PATH search makes an execve per miss.
            if !line.ends_with("= 0") {
                continue;
            }
            let program = quoted(line).unwrap_or_default();
            let program = program.rsplit('/').next().unwrap_or_default().to_string();
            if line.contains("\"list-panes\"") {
                ticks += 1;
                ticked.push(at);
            }
            ran.push(program);
        } else if line.contains("/sessions/") && line.contains(".json") {
            if call.starts_with("statx(") || call.starts_with("newfstatat(") {
                stats += 1;
            } else if call.starts_with("openat(") && line.contains("O_RDONLY") {
                reads += 1;
            }
        }
    }

    assert!(ticks > 0, "nothing ticked in ten seconds");
    let mut counted: Vec<(String, usize)> = Vec::new();
    for program in &ran {
        match counted.iter_mut().find(|(name, _)| name == program) {
            Some((_, seen)) => *seen += 1,
            None => counted.push((program.clone(), 1)),
        }
    }
    counted.sort_by_key(|(_, seen)| std::cmp::Reverse(*seen));
    let mine = ran
        .iter()
        .filter(|program| THE_APPS_OWN.contains(&program.as_str()))
        .count();
    // One rate per program as well as the total: the design's claim is about
    // the listing specifically.
    let each: Vec<String> = THE_APPS_OWN
        .iter()
        .map(|program| {
            let seen = ran.iter().filter(|run| run == program).count();

            format!("{program} {:.2}", how_many(seen) / f64::from(ticks))
        })
        .collect();

    let gaps: Vec<Duration> = ticked
        .windows(2)
        .filter_map(|pair| Duration::try_from_secs_f64(pair[1] - pair[0]).ok())
        .collect();
    said(&spread("the gap between one tick and the next", &gaps));
    said(&format!(
        "over {:.0} s of twenty spawns running, traced:\n\
         {ticks} ticks — {:.1} a second, against the {:?} the supervisor sleeps for\n\
         every program the trace saw run, and how many: {counted:?}\n\
         of which the app's own tools: {:.2} a tick, one at a time: {each:?}\n\
         {:.1} stats of a session record a tick, {:.2} of them read",
        to - from,
        f64::from(ticks) / (to - from),
        TICK,
        how_many(mine) / f64::from(ticks),
        f64::from(stats) / f64::from(ticks),
        f64::from(reads) / f64::from(ticks),
    ));
}

/// The whole of what the app shells out to; a program outside this list in a
/// tick's count would be worth knowing about.
const THE_APPS_OWN: [&str; 3] = ["tmux", "ps", "git"];

/// One line of a trace: which thread, when, and what it called.
fn traced_call(line: &str) -> Option<(&str, f64, &str)> {
    let mut fields = line.split_whitespace();
    let (thread, at, call) = (fields.next()?, fields.next()?, fields.next()?);

    Some((thread, at.parse().ok()?, call))
}

/// The one process a tracer is running, waited for until it is there.
fn traced_by(tracer: u32) -> u32 {
    let deadline = Instant::now() + PATIENCE;
    loop {
        let children = fs::read_to_string(format!("/proc/{tracer}/task/{tracer}/children"))
            .unwrap_or_default();
        if let Some(traced) = children.split_whitespace().next() {
            return traced.parse().unwrap();
        }
        assert!(
            Instant::now() < deadline,
            "the tracer never started anything"
        );
        thread::sleep(Duration::from_millis(10));
    }
}

/// The first double-quoted thing on a line — for an `execve`, the program.
fn quoted(line: &str) -> Option<String> {
    Some(line.split('"').nth(1)?.to_string())
}
