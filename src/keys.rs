//! A keystroke, as the spawn would have felt it.
//!
//! The app is the terminal the spawn is typed into, so every key has to be
//! turned back into the bytes a terminal would have sent. Pure, and worth being
//! pure: this is a table of conventions — some of them older than the terminals
//! that still speak them — and a table is a thing to read off rather than a
//! thing to trust to a running program.
//!
//! What it is *not* is a keymap. Which keys the app keeps for itself is settled
//! by whoever calls this; everything that reaches here is on its way to a spawn.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// The modes a spawn has put its terminal in that change what a key sends.
///
/// Read off the spawn's own screen rather than remembered here: the program
/// decides, mid-run, and a keyboard that guessed would send an arrow key that
/// moved nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Modes {
    /// Whether arrow keys are wanted in their application form.
    pub application_cursor: bool,
}

/// What a terminal would have sent for this key.
///
/// Nothing at all, for a key with no byte behind it — a bare modifier, or one of
/// the keys only terminals that have been told to report them ever send.
pub fn typed(key: KeyEvent, modes: Modes) -> Vec<u8> {
    let control = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);

    let mut bytes = match key.code {
        KeyCode::Char(character) if control => controlled(character),
        KeyCode::Char(character) => character.to_string().into_bytes(),
        KeyCode::Enter => vec![b'\r'],
        KeyCode::Tab => vec![b'\t'],
        KeyCode::BackTab => b"\x1b[Z".to_vec(),
        KeyCode::Backspace => vec![0x7f],
        KeyCode::Esc => vec![0x1b],
        KeyCode::Up => arrow(b'A', modes),
        KeyCode::Down => arrow(b'B', modes),
        KeyCode::Right => arrow(b'C', modes),
        KeyCode::Left => arrow(b'D', modes),
        KeyCode::Home => arrow(b'H', modes),
        KeyCode::End => arrow(b'F', modes),
        KeyCode::Insert => tilde(2),
        KeyCode::Delete => tilde(3),
        KeyCode::PageUp => tilde(5),
        KeyCode::PageDown => tilde(6),
        KeyCode::F(number) => function(number),
        _ => Vec::new(),
    };

    // Alt is a prefix rather than a key: the terminal sends escape, then
    // whatever the key would have sent on its own.
    if alt && !bytes.is_empty() {
        bytes.insert(0, 0x1b);
    }

    bytes
}

/// What holding control turns a character into.
///
/// The bottom five bits of the upper-case letter, which is why control and the
/// letter it is held with are one byte rather than two — and why interrupting a
/// spawn is `03` and nothing more elaborate.
fn controlled(character: char) -> Vec<u8> {
    let upper = character.to_ascii_uppercase();
    match upper {
        '@'..='_' => vec![upper as u8 & 0x1f],
        '?' => vec![0x7f],
        ' ' => vec![0],
        _ => character.to_string().into_bytes(),
    }
}

/// An arrow or a corner, in whichever form the spawn asked for.
///
/// The same key, two escape sequences: programs that draw a full screen usually
/// ask for the application form, and a terminal that sends the other one has
/// keys that do nothing.
fn arrow(letter: u8, modes: Modes) -> Vec<u8> {
    let introducer = if modes.application_cursor { b'O' } else { b'[' };

    vec![0x1b, introducer, letter]
}

/// One of the keys that are a number and a tilde.
fn tilde(number: u8) -> Vec<u8> {
    format!("\x1b[{number}~").into_bytes()
}

/// A function key, in the two shapes they come in.
///
/// The first four are the terminal's own; the rest are numbered, and the
/// numbering skips — which is a historical accident rather than a pattern, and
/// the reason this is a table.
fn function(number: u8) -> Vec<u8> {
    match number {
        1..=4 => vec![0x1b, b'O', b'P' + (number - 1)],
        5 => tilde(15),
        6..=10 => tilde(17 + (number - 6)),
        11..=12 => tilde(23 + (number - 11)),
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::crossterm::event::KeyEvent;

    /// The modes an ordinary terminal starts in.
    const PLAIN: Modes = Modes {
        application_cursor: false,
    };

    /// The modes a program drawing a whole screen usually asks for.
    const APPLICATION: Modes = Modes {
        application_cursor: true,
    };

    fn sent(code: KeyCode) -> Vec<u8> {
        typed(KeyEvent::from(code), PLAIN)
    }

    fn sent_with(code: KeyCode, modifiers: KeyModifiers) -> Vec<u8> {
        typed(KeyEvent::new(code, modifiers), PLAIN)
    }

    #[test]
    fn typing_a_character_sends_that_character() {
        assert_eq!(sent(KeyCode::Char('a')), b"a");
        assert_eq!(sent(KeyCode::Char('世')), "世".as_bytes());
    }

    #[test]
    fn interrupting_a_spawn_is_one_byte() {
        assert_eq!(
            sent_with(KeyCode::Char('c'), KeyModifiers::CONTROL),
            vec![0x03]
        );
        assert_eq!(
            sent_with(KeyCode::Char('C'), KeyModifiers::CONTROL),
            vec![0x03],
            "holding shift as well changed what interrupting means"
        );
    }

    #[test]
    fn the_keys_a_prompt_is_answered_with_are_what_a_terminal_sends() {
        assert_eq!(sent(KeyCode::Enter), b"\r");
        assert_eq!(sent(KeyCode::Backspace), vec![0x7f]);
        assert_eq!(sent(KeyCode::Esc), vec![0x1b]);
        assert_eq!(sent(KeyCode::Tab), b"\t");
    }

    #[test]
    fn an_arrow_key_follows_the_mode_the_spawn_asked_for() {
        assert_eq!(typed(KeyEvent::from(KeyCode::Up), PLAIN), b"\x1b[A");
        assert_eq!(typed(KeyEvent::from(KeyCode::Up), APPLICATION), b"\x1bOA");
        assert_eq!(typed(KeyEvent::from(KeyCode::Left), PLAIN), b"\x1b[D");
    }

    #[test]
    fn the_keys_that_are_a_number_and_a_tilde_carry_their_number() {
        assert_eq!(sent(KeyCode::Delete), b"\x1b[3~");
        assert_eq!(sent(KeyCode::PageUp), b"\x1b[5~");
        assert_eq!(sent(KeyCode::PageDown), b"\x1b[6~");
    }

    #[test]
    fn a_function_key_is_read_off_the_table_rather_than_worked_out() {
        assert_eq!(sent(KeyCode::F(1)), b"\x1bOP");
        assert_eq!(sent(KeyCode::F(4)), b"\x1bOS");
        assert_eq!(sent(KeyCode::F(5)), b"\x1b[15~");
        assert_eq!(sent(KeyCode::F(6)), b"\x1b[17~");
        assert_eq!(sent(KeyCode::F(12)), b"\x1b[24~");
    }

    #[test]
    fn holding_alt_puts_an_escape_in_front_of_what_the_key_would_send() {
        assert_eq!(sent_with(KeyCode::Char('b'), KeyModifiers::ALT), b"\x1bb");
        assert_eq!(sent_with(KeyCode::Enter, KeyModifiers::ALT), b"\x1b\r");
    }

    #[test]
    fn a_key_with_nothing_behind_it_sends_nothing_rather_than_something_odd() {
        assert!(sent(KeyCode::F(13)).is_empty());
        assert!(sent(KeyCode::CapsLock).is_empty());
        assert!(sent_with(KeyCode::CapsLock, KeyModifiers::ALT).is_empty());
    }
}
