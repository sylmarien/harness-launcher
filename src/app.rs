//! The list pane.
//!
//! One spawn, and it is already in the slot. What the row says about it is not
//! static any more: the supervisor works out what every spawn is doing and
//! sends a snapshot down a channel, and this draws the latest one it has been
//! given. Nothing is shared between the two — a snapshot is built whole and
//! handed over, so a row is never read while it is being written.
//!
//! Nothing here is a fixed size. Every dimension comes from the real pane on
//! every frame, so a maximised terminal is a bigger layout rather than a bigger
//! frame around a small one.

use std::io;
use std::sync::mpsc::Receiver;
use std::time::Duration;

use ratatui::Frame;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::layout::Alignment;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};

use crate::error::{Error, Result};
use crate::snapshot::{Row, Snapshot, Status};

/// How long a frame waits for a keystroke before drawing itself again.
const TICK: Duration = Duration::from_millis(200);

/// The colour reserved for the app failing to know something.
const AMBER: Color = Color::Yellow;

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

/// Draw the list until the user quits.
///
/// Snapshots are drained rather than queued: what the user wants to see is what
/// is true now, so a frame that arrives behind several ticks skips them.
pub fn run(view: &View, snapshots: &Receiver<Snapshot>) -> Result<()> {
    let mut latest = Snapshot::default();

    ratatui::run(|terminal| -> io::Result<()> {
        loop {
            while let Ok(snapshot) = snapshots.try_recv() {
                latest = snapshot;
            }
            terminal.draw(|frame| render(frame, view, &latest))?;
            if quit_requested()? {
                return Ok(());
            }
        }
    })
    .map_err(|error| Error::new(format!("the list pane stopped: {error}")))
}

/// Paint one frame.
pub fn render(frame: &mut Frame, view: &View, snapshot: &Snapshot) {
    let dim = Style::default().add_modifier(Modifier::DIM);
    let heading = Style::default().add_modifier(Modifier::BOLD);
    let spawn = snapshot.of(&view.spawn);
    let (mark, how_it_reads) = shown_as(spawn);

    let mut lines = vec![
        Line::styled("SPAWNS", heading),
        Line::raw(""),
        Line::styled(view.repository.clone(), heading),
        Line::from(vec![
            Span::raw("▍"),
            Span::styled(mark, how_it_reads),
            Span::styled(view.spawn.clone(), how_it_reads),
        ]),
        Line::styled(format!("  {}", view.branch), dim),
        Line::styled(format!("  {}", view.worktree), dim),
    ];
    if let Some(why) = spawn.and_then(|row| row.reason.as_ref()) {
        lines.push(Line::styled(format!("  {why}"), dim.fg(AMBER)));
    }
    lines.extend([
        Line::raw(""),
        Line::styled(
            "the slot on the right is a real session — your keyboard is already on it",
            dim,
        ),
        Line::styled("q here quits the app and leaves the session running", dim),
    ]);

    frame.render_widget(
        Paragraph::new(lines)
            .alignment(Alignment::Left)
            .wrap(Wrap { trim: false }),
        frame.area(),
    );
}

/// The mark a status is carried by, and how the row reads in it.
///
/// One answer rather than two, because the mark and the colour have to travel
/// together: at twenty entries the list must read without a legend and survive
/// a colour-blind reader, and a shape and a colour decided in separate places
/// are two things that can come to disagree.
///
/// Working recedes, stopped is the only bright thing, unknown is the outlier. A
/// spawn the app has not heard about yet is a blank of the same width, so a row
/// does not shift sideways when the first snapshot lands.
fn shown_as(row: Option<&Row>) -> (&'static str, Style) {
    match row.map(|row| row.status) {
        Some(Status::Working) => ("· ", Style::default().add_modifier(Modifier::DIM)),
        Some(Status::Stopped) => ("● ", Style::default().add_modifier(Modifier::BOLD)),
        Some(Status::Unknown) => ("? ", Style::default().fg(AMBER)),
        None => ("  ", Style::default()),
    }
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

    /// What the supervisor would have said about the one spawn there is.
    fn saying(status: Status, reason: Option<&str>) -> Snapshot {
        Snapshot {
            rows: vec![Row {
                name: view().spawn,
                status,
                reason: reason.map(str::to_string),
            }],
        }
    }

    fn rendered(width: u16, height: u16) -> String {
        drawn(width, height, &saying(Status::Working, None))
    }

    fn drawn(width: u16, height: u16, snapshot: &Snapshot) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| render(frame, &view(), snapshot))
            .unwrap();
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

    /// The row the spawn is on, whatever else moved around it.
    fn row(screen: &str) -> String {
        screen
            .lines()
            .find(|line| line.contains("add-retry-logic-a7f3"))
            .unwrap_or_else(|| panic!("the spawn is not on screen:\n{screen}"))
            .to_string()
    }

    #[test]
    fn each_status_is_a_mark_of_its_own_beside_the_spawn() {
        let working = row(&drawn(60, 12, &saying(Status::Working, None)));
        let stopped = row(&drawn(60, 12, &saying(Status::Stopped, None)));
        let unknown = row(&drawn(60, 12, &saying(Status::Unknown, Some("no record"))));

        assert!(working.contains('·'), "{working}");
        assert!(stopped.contains('●'), "{stopped}");
        assert!(unknown.contains('?'), "{unknown}");
        assert_ne!(working.trim(), stopped.trim());
        assert_ne!(stopped.trim(), unknown.trim());
    }

    #[test]
    fn a_spawn_the_app_cannot_tell_about_says_why_on_screen() {
        let screen = drawn(
            60,
            14,
            &saying(
                Status::Unknown,
                Some("its session record carries no status"),
            ),
        );

        assert!(screen.contains("carries no status"), "{screen}");
    }

    /// How many lines of the screen have anything on them.
    fn written(screen: &str) -> usize {
        screen
            .lines()
            .filter(|line| !line.trim().is_empty())
            .count()
    }

    #[test]
    fn a_reason_takes_a_line_only_when_there_is_something_to_explain() {
        let explained = drawn(60, 14, &saying(Status::Unknown, Some("no record")));
        let plain = drawn(60, 14, &saying(Status::Working, None));

        assert_eq!(
            written(&explained),
            written(&plain) + 1,
            "an explained row and an unexplained one are the same height:\n{explained}"
        );
    }

    #[test]
    fn before_the_first_snapshot_the_row_claims_nothing() {
        let screen = drawn(60, 12, &Snapshot::default());
        let row = row(&screen);

        assert!(!row.contains('·'), "{row}");
        assert!(!row.contains('●'), "{row}");
        assert!(!row.contains('?'), "{row}");
        assert!(row.contains("add-retry-logic-a7f3"), "{row}");
    }

    /// Which column something starts in — cells, not bytes, since a mark and a
    /// blank of the same width are not the same number of bytes.
    fn column_of(row: &str, text: &str) -> usize {
        let at = row
            .find(text)
            .unwrap_or_else(|| panic!("{text} is not on the row: {row}"));

        row[..at].chars().count()
    }

    #[test]
    fn a_spawn_moves_between_statuses_without_moving_on_screen() {
        let columns: Vec<usize> = [
            saying(Status::Working, None),
            saying(Status::Stopped, None),
            saying(Status::Unknown, Some("no record")),
            Snapshot::default(),
        ]
        .iter()
        .map(|snapshot| column_of(&row(&drawn(60, 12, snapshot)), "add-retry-logic-a7f3"))
        .collect();

        assert!(
            columns.windows(2).all(|pair| pair[0] == pair[1]),
            "the name shifted sideways as the status changed: {columns:?}"
        );
    }
}
