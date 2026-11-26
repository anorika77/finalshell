use vt100::Parser;

/// Stateful VT terminal model, separate from the SSH transport and UI pane.
pub struct TerminalScreen {
    parser: Parser,
    rows: u16,
    cols: u16,
}

impl TerminalScreen {
    pub fn new(rows: u16, cols: u16, scrollback_capacity: usize) -> Self {
        let rows = rows.max(1);
        let cols = cols.max(1);
        Self {
            parser: Parser::new(rows, cols, scrollback_capacity),
            rows,
            cols,
        }
    }

    /// Feed raw terminal bytes. Keep decoding out of this layer so the
    /// transport and parser never corrupt split multibyte sequences.
    pub fn process(&mut self, bytes: &[u8]) {
        self.parser.process(bytes);
    }

    pub fn resize(&mut self, rows: u16, cols: u16) {
        self.rows = rows.max(1);
        self.cols = cols.max(1);
        self.parser.set_size(self.rows, self.cols);
    }

    /// Scrollback offset: zero is the live viewport.
    pub fn set_scrollback(&mut self, offset: usize) {
        self.parser.set_scrollback(offset);
    }

    pub fn scrollback_offset(&self) -> usize {
        self.parser.screen().scrollback()
    }

    pub fn dimensions(&self) -> (u16, u16) {
        (self.rows, self.cols)
    }

    pub fn cursor_position(&self) -> (u16, u16) {
        self.parser.screen().cursor_position()
    }

    pub fn contents(&self) -> String {
        self.parser.screen().contents()
    }

    /// ANSI-formatted visible screen, suitable for a renderer that supports
    /// the terminal's SGR attributes.
    pub fn contents_formatted(&self) -> Vec<u8> {
        self.parser.screen().contents_formatted()
    }

    pub fn alternate_screen(&self) -> bool {
        self.parser.screen().alternate_screen()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_text_and_cursor_position() {
        let mut screen = TerminalScreen::new(4, 20, 100);
        screen.process(b"hello\r\nworld");
        assert!(screen.contents().contains("hello"));
        assert!(screen.contents().contains("world"));
        assert_eq!(screen.cursor_position(), (1, 5));
    }

    #[test]
    fn parses_ansi_colors_and_formatted_output() {
        let mut screen = TerminalScreen::new(2, 20, 0);
        screen.process(b"\\x1b[31mred\\x1b[0m");
        let cell = screen.parser.screen().cell(0, 0).unwrap();
        assert_eq!(cell.fgcolor(), vt100::Color::Idx(1));
        assert!(!screen.contents_formatted().is_empty());
    }

    #[test]
    fn resize_clamps_zero_dimensions() {
        let mut screen = TerminalScreen::new(0, 0, 0);
        screen.resize(0, 0);
        assert_eq!(screen.dimensions(), (1, 1));
        assert_eq!(screen.parser.screen().size(), (1, 1));
    }

    #[test]
    fn alternate_screen_mode_is_tracked() {
        let mut screen = TerminalScreen::new(2, 10, 10);
        screen.process(b"\\x1b[?1049h");
        assert!(screen.alternate_screen());
        screen.process(b"\\x1b[?1049l");
        assert!(!screen.alternate_screen());
    }
}
