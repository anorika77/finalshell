use ghostty_vt::{Event, Terminal, TerminalOptions};

fn main() {
    let mut terminal = Terminal::new(TerminalOptions {
        cols: 80,
        rows: 24,
        ..Default::default()
    })
    .expect("create Ghostty terminal");

    terminal.vt_write(b"FinalShell terminal spike\r\n\x1b[1;32mVT parser online\x1b[0m\r\n");

    println!("libghostty-vt terminal initialized and accepted VT input");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_vt_stream_and_emits_side_effect_events() {
        let mut terminal = Terminal::new(TerminalOptions {
            cols: 80,
            rows: 24,
            ..Default::default()
        })
        .expect("create terminal");

        terminal.vt_write(
            b"plain text\r\n\x1b[31mred text\x1b[0m\r\n\x07\x1b]2;terminal-spike-title\x07",
        );

        let events: Vec<Event> = terminal.events().collect();

        assert!(events.iter().any(|event| matches!(event, Event::Bell)));
        assert!(events.iter().any(
            |event| matches!(event, Event::TitleChanged(title) if title == "terminal-spike-title")
        ));
    }

    #[test]
    fn accepts_resize_and_unicode_input() {
        let mut terminal = Terminal::new(TerminalOptions {
            cols: 80,
            rows: 24,
            ..Default::default()
        })
        .expect("create terminal");

        terminal.vt_write("中文宽字符 e\u{301} emoji 🦀\r\n".as_bytes());
        terminal.resize(120, 40, 8, 16).expect("resize terminal");
    }
}
