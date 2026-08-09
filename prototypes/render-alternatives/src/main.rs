//! PROTOTYPE — throwaway. Not production code.
//!
//! Tests two alternatives to settled decisions:
//!
//!   --backend pty      (B) we own the pty; tmux is not involved at all
//!   --backend tmux-cc  (A) tmux still owns the process, in CONTROL MODE, but
//!                          we receive its output and render it ourselves
//!
//! Both share one renderer, so the difference under test is purely *who owns
//! the process and how the bytes reach us* — not how they are drawn.
//!
//! The renderer is the real Rust stack the design rejected: vt100 for terminal
//! emulation, drawn into ratatui cells. That matters — vt100 has a specific
//! known limitation (it cannot write back to the child, so terminal queries
//! like cursor-position reports go unanswered) and using a friendlier emulator
//! from another ecosystem would give a falsely optimistic answer.
//!
//!   cargo run -- --backend pty
//!   cargo run -- --backend tmux-cc
//!   cargo run -- --backend pty --n 4
//!   cargo run -- --backend pty --cmd htop      # rehearse without tokens
//!
//! F1..F9 switch between children. F10 quits. Everything else goes to the
//! selected child. Function keys rather than digits, because a digit is exactly
//! what you need to send when Claude Code asks you to pick an option.
//!
//! Note what switching costs here: nothing. Every child keeps its own grid in
//! memory, so selecting another one is a different render, not a resize. That
//! is the substantive contrast with the parking mechanism in the current
//! design, where every switch resizes the child and forces a full repaint.

use std::io::{Read, Write};
use std::sync::{Arc, Mutex};

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph};

const SOCKET: &str = "hlrender";

type Screen = Arc<Mutex<vt100::Parser>>;

/// Anything that can carry keystrokes to the child.
trait Input: Send {
    fn send(&mut self, bytes: &[u8]);
}

// ---------------------------------------------------------------- pty backend
struct PtyInput(Box<dyn Write + Send>);
impl Input for PtyInput {
    fn send(&mut self, b: &[u8]) {
        let _ = self.0.write_all(b);
        let _ = self.0.flush();
    }
}

// ------------------------------------------------------------ tmux -CC input
struct CcInput {
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    pane: String,
}
impl Input for CcInput {
    fn send(&mut self, b: &[u8]) {
        // send-keys -H takes hex bytes; the only encoding that survives
        // arbitrary keystrokes without quoting games.
        let hex: Vec<String> = b.iter().map(|x| format!("{x:02x}")).collect();
        let mut w = self.writer.lock().unwrap();
        let _ = writeln!(w, "send-keys -t {} -H {}", self.pane, hex.join(" "));
        let _ = w.flush();
    }
}

/// tmux escapes non-printables in %output as octal: `\033`.
fn unescape(s: &str) -> Vec<u8> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\\' && i + 3 < b.len() {
            let oct = std::str::from_utf8(&b[i + 1..i + 4]).unwrap_or("");
            if let Ok(v) = u8::from_str_radix(oct, 8) {
                out.push(v);
                i += 4;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    out
}

/// One child under test: its screen, and a way to type at it.
struct Child {
    screen: Screen,
    input: Box<dyn Input>,
    label: String,
}

fn start_pty_children(
    n: usize,
    cmd: &str,
    args: &[String],
    rows: u16,
    cols: u16,
) -> Vec<Child> {
    use portable_pty::{native_pty_system, CommandBuilder, PtySize};
    let mut out = Vec::new();
    for i in 0..n {
        let screen: Screen = Arc::new(Mutex::new(vt100::Parser::new(rows, cols, 0)));
        let pty = native_pty_system()
            .openpty(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 })
            .expect("openpty");
        let mut builder = CommandBuilder::new(cmd);
        for a in args {
            builder.arg(a);
        }
        builder.env("TERM", "xterm-256color");
        // Claude Code picks its renderer from this; force fullscreen as the design does.
        builder.env("CLAUDE_CODE_NO_FLICKER", "1");
        let _child = pty.slave.spawn_command(builder).expect("spawn");
        drop(pty.slave);

        let mut reader = pty.master.try_clone_reader().expect("reader");
        let sc = screen.clone();
        std::thread::spawn(move || {
            let mut buf = [0u8; 8192];
            while let Ok(k) = reader.read(&mut buf) {
                if k == 0 {
                    break;
                }
                sc.lock().unwrap().process(&buf[..k]);
            }
        });
        out.push(Child {
            screen,
            input: Box::new(PtyInput(pty.master.take_writer().expect("writer"))),
            label: format!("spawn-{}", i + 1),
        });
    }
    out
}

fn start_tmux_cc_children(
    n: usize,
    cmd: &str,
    args: &[String],
    rows: u16,
    cols: u16,
) -> Vec<Child> {
    use portable_pty::{native_pty_system, CommandBuilder, PtySize};
    use std::process::Command;

    let _ = Command::new("tmux").args(["-L", SOCKET, "kill-server"]).status();

    let mut full = cmd.to_string();
    for a in args {
        full.push(' ');
        full.push_str(a);
    }

    // Start every pane with a HOLDER, not the real command.
    //
    // Verified against tmux 3.4: control mode streams only what is produced
    // *while a control client is attached*. Attaching after a child has drawn
    // itself leaves the client staring at a blank screen with no way to catch
    // up short of priming from `capture-pane`. So: hold the panes open,
    // attach, and only then respawn each with the real command.
    let holder = "while :; do sleep 3600; done";
    Command::new("tmux")
        .args([
            "-L", SOCKET, "new-session", "-d", "-s", "p", "-n", "s1",
            "-x", &cols.to_string(), "-y", &rows.to_string(),
            "sh", "-c", holder,
        ])
        .status()
        .expect("tmux new-session");
    for i in 1..n {
        Command::new("tmux")
            .args([
                "-L", SOCKET, "new-window", "-d", "-t", "p",
                "-n", &format!("s{}", i + 1), "sh", "-c", holder,
            ])
            .status()
            .expect("tmux new-window");
    }

    // A control client needs a tty of its own. Verified: with piped stdio tmux
    // refuses outright — "tcgetattr failed: Inappropriate ioctl for device".
    let pty = native_pty_system()
        .openpty(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 })
        .expect("openpty for control client");
    let mut builder = CommandBuilder::new("tmux");
    builder.args(["-L", SOCKET, "-CC", "attach", "-t", "p"]);
    builder.env("TERM", "xterm-256color");
    let _client = pty.slave.spawn_command(builder).expect("spawn control client");
    drop(pty.slave);

    let reader = pty.master.try_clone_reader().expect("reader");
    let writer: Arc<Mutex<Box<dyn Write + Send>>> =
        Arc::new(Mutex::new(pty.master.take_writer().expect("writer")));

    // One control client carries every pane in the session, including windows
    // that are not visible. Verified on tmux 3.4: pane ids are %0..%(n-1) in
    // creation order on a fresh server, which is why the server is killed above.
    let mut children = Vec::new();
    let mut routes: Vec<(String, Screen)> = Vec::new();
    for i in 0..n {
        let screen: Screen = Arc::new(Mutex::new(vt100::Parser::new(rows, cols, 0)));
        let pane = format!("%{i}");
        routes.push((pane.clone(), screen.clone()));
        children.push(Child {
            screen,
            input: Box::new(CcInput { writer: writer.clone(), pane }),
            label: format!("spawn-{}", i + 1),
        });
    }

    std::thread::spawn(move || {
        use std::io::{BufRead, BufReader};
        let mut lines = BufReader::new(reader);
        let mut line = String::new();
        loop {
            line.clear();
            match lines.read_line(&mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) => {}
            }
            // Strip the terminator; the pty gives CRLF, and a stray CR would
            // otherwise reach the emulator as real data.
            let l = line.trim_end_matches(['\n', '\r']);
            let Some(rest) = l.strip_prefix("%output ") else { continue };
            let Some(sp) = rest.find(' ') else { continue };
            let pane = &rest[..sp];
            if let Some((_, sc)) = routes.iter().find(|(p, _)| p == pane) {
                sc.lock().unwrap().process(&unescape(&rest[sp + 1..]));
            }
        }
    });

    // Let the client finish attaching, then start the real commands so every
    // byte they emit arrives as %output.
    std::thread::sleep(std::time::Duration::from_millis(400));
    {
        let mut w = writer.lock().unwrap();
        let _ = writeln!(w, "refresh-client -C {cols},{rows}");
        for i in 0..n {
            let _ = writeln!(
                w,
                "respawn-pane -k -t %{i} 'CLAUDE_CODE_NO_FLICKER=1 {}'",
                full.replace('\'', "'\\''")
            );
        }
        let _ = w.flush();
    }

    children
}

// ------------------------------------------------------------------- rendering
/// Draw the vt100 grid into ratatui cells by hand.
///
/// Deliberately not using `tui-term`: doing it manually keeps the experiment
/// about vt100's fidelity rather than about a wrapper's version compatibility.
fn draw_screen(f: &mut Frame, area: Rect, parser: &vt100::Parser) {
    let screen = parser.screen();
    let buf = f.buffer_mut();
    for row in 0..area.height {
        for col in 0..area.width {
            let Some(cell) = screen.cell(row, col) else { continue };
            let x = area.x + col;
            let y = area.y + row;
            let mut style = Style::default();
            if let Some(c) = conv(cell.fgcolor()) {
                style = style.fg(c);
            }
            if let Some(c) = conv(cell.bgcolor()) {
                style = style.bg(c);
            }
            if cell.bold() {
                style = style.add_modifier(Modifier::BOLD);
            }
            if cell.italic() {
                style = style.add_modifier(Modifier::ITALIC);
            }
            if cell.inverse() {
                style = style.add_modifier(Modifier::REVERSED);
            }
            let contents = cell.contents();
            let s = if contents.is_empty() { " ".to_string() } else { contents };
            buf[(x, y)].set_symbol(&s).set_style(style);
        }
    }
}

fn conv(c: vt100::Color) -> Option<Color> {
    match c {
        vt100::Color::Default => None,
        vt100::Color::Idx(i) => Some(Color::Indexed(i)),
        vt100::Color::Rgb(r, g, b) => Some(Color::Rgb(r, g, b)),
    }
}

fn key_bytes(k: KeyEvent) -> Vec<u8> {
    let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
    match k.code {
        KeyCode::Char(c) if ctrl => vec![(c.to_ascii_uppercase() as u8) & 0x1f],
        KeyCode::Char(c) => c.to_string().into_bytes(),
        KeyCode::Enter => vec![b'\r'],
        KeyCode::Backspace => vec![0x7f],
        KeyCode::Tab => vec![b'\t'],
        KeyCode::Esc => vec![0x1b],
        KeyCode::Up => b"\x1b[A".to_vec(),
        KeyCode::Down => b"\x1b[B".to_vec(),
        KeyCode::Right => b"\x1b[C".to_vec(),
        KeyCode::Left => b"\x1b[D".to_vec(),
        _ => vec![],
    }
}

fn main() {
    let argv: Vec<String> = std::env::args().collect();
    let get = |name: &str, default: &str| -> String {
        argv.iter()
            .position(|a| a == name)
            .and_then(|i| argv.get(i + 1))
            .cloned()
            .unwrap_or_else(|| default.to_string())
    };
    let backend = get("--backend", "pty");
    let cmd = get("--cmd", "claude");
    let n: usize = get("--n", "3").parse().unwrap_or(3).clamp(1, 9);
    let extra: Vec<String> = argv
        .iter()
        .position(|a| a == "--")
        .map(|i| argv[i + 1..].to_vec())
        .unwrap_or_default();

    let (term_cols, term_rows) = crossterm::terminal::size().unwrap_or((120, 40));
    let child_cols = term_cols.saturating_sub(term_cols / 3).saturating_sub(2).max(20);
    let child_rows = term_rows.saturating_sub(2).max(10);

    let mut children = match backend.as_str() {
        "tmux-cc" => start_tmux_cc_children(n, &cmd, &extra, child_rows, child_cols),
        _ => start_pty_children(n, &cmd, &extra, child_rows, child_cols),
    };
    let mut sel: usize = 0;

    enable_raw_mode().unwrap();
    let mut out = std::io::stdout();
    crossterm::execute!(out, EnterAlternateScreen).unwrap();
    let mut term = Terminal::new(CrosstermBackend::new(out)).unwrap();

    let label = if backend == "tmux-cc" {
        "(A) tmux control mode — tmux owns them, we render"
    } else {
        "(B) own pty — no tmux at all"
    };

    loop {
        term.draw(|f| {
            let cols = Layout::horizontal([Constraint::Percentage(32), Constraint::Percentage(68)])
                .split(f.area());

            let mut lines = vec![
                Line::from(Span::styled(
                    "SPAWNS",
                    Style::default().add_modifier(Modifier::BOLD),
                )),
                Line::from(""),
            ];
            for (i, c) in children.iter().enumerate() {
                let selected = i == sel;
                let marker = if selected { "▍● " } else { "  · " };
                let style = if selected {
                    Style::default().fg(Color::White).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::DarkGray)
                };
                lines.push(Line::from(Span::styled(
                    format!("{marker}{}   F{}", c.label, i + 1),
                    style,
                )));
            }
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(label, Style::default().fg(Color::Cyan))));
            lines.push(Line::from(""));
            for hint in [
                "F1..F9  switch child",
                "F10     quit",
                "",
                "Switching is a re-render,",
                "not a resize: every child",
                "keeps its own grid, so",
                "nothing repaints.",
            ] {
                lines.push(Line::from(Span::styled(
                    hint,
                    Style::default().fg(Color::DarkGray),
                )));
            }
            f.render_widget(
                Paragraph::new(lines).block(Block::default().borders(Borders::RIGHT)),
                cols[0],
            );

            let inner = cols[1];
            f.render_widget(Block::default(), inner);
            let parser = children[sel].screen.lock().unwrap();
            draw_screen(f, inner, &parser);
            let (cr, cc) = parser.screen().cursor_position();
            if cr < inner.height && cc < inner.width {
                f.set_cursor_position((inner.x + cc, inner.y + cr));
            }
        })
        .unwrap();

        if event::poll(std::time::Duration::from_millis(16)).unwrap_or(false) {
            if let Ok(Event::Key(k)) = event::read() {
                match k.code {
                    KeyCode::F(10) => break,
                    KeyCode::F(i) if (1..=9).contains(&i) => {
                        let idx = (i - 1) as usize;
                        if idx < children.len() {
                            sel = idx;
                        }
                    }
                    _ => {
                        let b = key_bytes(k);
                        if !b.is_empty() {
                            children[sel].input.send(&b);
                        }
                    }
                }
            }
        }
    }

    disable_raw_mode().unwrap();
    crossterm::execute!(term.backend_mut(), LeaveAlternateScreen).unwrap();
    if backend == "tmux-cc" {
        let _ = std::process::Command::new("tmux")
            .args(["-L", SOCKET, "kill-server"])
            .status();
    }
}
