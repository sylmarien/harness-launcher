//! The list pane.
//!
//! One spawn, and it is already in the slot — the list this draws is static
//! text, and deliberately so: this pane exists to prove the window is composed
//! correctly, not yet to say anything live about what the session is doing.
//!
//! Nothing here is a fixed size. Every dimension comes from the real pane on
//! every frame, so a maximised terminal is a bigger layout rather than a bigger
//! frame around a small one.

use std::io;
use std::time::Duration;

use ratatui::Frame;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::layout::Alignment;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};

use crate::error::{Error, Result};

/// How long a frame waits for a keystroke before drawing itself again.
const TICK: Duration = Duration::from_millis(200);

/// What the list pane has to say.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct View {
    /// The repository the spawn was started against.
    pub repository: String,
    /// The spawn's name.
    pub spawn: String,
    /// The branch it works on.
    pub branch: String,
    /// The worktree it works in.
    pub worktree: String,
}

/// How big the terminal the app was started in is, when it will say.
///
/// Not knowing is not a refusal. Some pseudo-terminals report nothing usable,
/// and tmux sizes a window when a client attaches to it anyway — the size is
/// asked for so that the first thing the user sees is already the right shape,
/// not because the app could not manage without it.
pub fn terminal_size() -> Option<(u16, u16)> {
    ratatui::crossterm::terminal::size()
        .ok()
        .filter(|(columns, rows)| *columns > 0 && *rows > 0)
}

/// Draw the list until the user quits.
pub fn run(view: &View) -> Result<()> {
    ratatui::run(|terminal| -> io::Result<()> {
        loop {
            terminal.draw(|frame| render(frame, view))?;
            if quit_requested()? {
                return Ok(());
            }
        }
    })
    .map_err(|error| Error::new(format!("the list pane stopped: {error}")))
}

/// Paint one frame.
pub fn render(frame: &mut Frame, view: &View) {
    let dim = Style::default().add_modifier(Modifier::DIM);
    let heading = Style::default().add_modifier(Modifier::BOLD);

    let lines = vec![
        Line::styled("SPAWNS", heading),
        Line::raw(""),
        Line::styled(view.repository.clone(), heading),
        Line::from(vec![Span::raw("▍"), Span::raw(view.spawn.clone())]),
        Line::styled(format!("  {}", view.branch), dim),
        Line::styled(format!("  {}", view.worktree), dim),
        Line::raw(""),
        Line::styled(
            "the slot on the right is a real session — your keyboard is already on it",
            dim,
        ),
        Line::styled("q here quits the app and leaves the session running", dim),
    ];

    frame.render_widget(
        Paragraph::new(lines)
            .alignment(Alignment::Left)
            .wrap(Wrap { trim: false }),
        frame.area(),
    );
}

/// Whether the user asked to leave.
fn quit_requested() -> io::Result<bool> {
    if !event::poll(TICK)? {
        return Ok(false);
    }

    let Event::Key(key) = event::read()? else {
        return Ok(false);
    };
    if key.kind != KeyEventKind::Press {
        return Ok(false);
    }

    Ok(matches!(key.code, KeyCode::Char('q') | KeyCode::Esc)
        || (key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn view() -> View {
        View {
            repository: "harness-launcher".to_string(),
            spawn: "add-retry-logic-a7f3".to_string(),
            branch: "spawn/add-retry-logic-a7f3".to_string(),
            worktree: "/data/harness-launcher/worktrees/add-retry-logic-a7f3".to_string(),
        }
    }

    fn rendered(width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| render(frame, &view())).unwrap();
        let buffer = terminal.backend().buffer().clone();

        (0..buffer.area.height)
            .map(|row| {
                (0..buffer.area.width)
                    .map(|column| buffer[(column, row)].symbol().to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn the_list_names_the_spawn_under_its_repository() {
        let screen = rendered(40, 12);

        let repository = screen.find("harness-launcher").unwrap();
        let spawn = screen.find("add-retry-logic-a7f3").unwrap();
        assert!(
            repository < spawn,
            "the repository heads its spawns:\n{screen}"
        );
    }

    #[test]
    fn the_list_says_what_the_app_created() {
        let screen = rendered(60, 12);

        assert!(screen.contains("spawn/add-retry-logic-a7f3"), "{screen}");
        assert!(
            screen.contains("/data/harness-launcher/worktrees"),
            "{screen}"
        );
    }

    #[test]
    fn a_narrow_pane_wraps_rather_than_losing_the_text() {
        let screen = rendered(24, 24);

        assert!(screen.contains("add-retry-logic-a7f3"), "{screen}");
        assert!(screen.contains("quits the app"), "{screen}");
    }

    #[test]
    fn a_wide_pane_is_not_a_frame_around_a_narrow_one() {
        let wide = rendered(120, 12);

        let longest = wide.lines().map(str::trim_end).map(str::len).max().unwrap();
        assert!(longest > 60, "the layout did not use the width:\n{wide}");
    }

    #[test]
    fn a_pane_too_short_for_everything_still_draws() {
        let screen = rendered(40, 3);

        assert!(screen.contains("SPAWNS"), "{screen}");
    }
}
