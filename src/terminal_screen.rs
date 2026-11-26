use encoding_rs::{BIG5, Encoding, GBK, UTF_8};
use vt100::Parser;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalCharset {
    Utf8,
    Gbk,
    Big5,
}

impl TerminalCharset {
    fn encoding(self) -> &'static Encoding {
        match self {
            Self::Utf8 => UTF_8,
            Self::Gbk => GBK,
            Self::Big5 => BIG5,
        }
    }
}

pub struct TerminalDecoder {
    charset: TerminalCharset,
    decoder: Option<encoding_rs::Decoder>,
}

impl TerminalDecoder {
    pub fn new(charset: TerminalCharset) -> Self {
        Self {
            charset,
            decoder: None,
        }
    }

    pub fn set_charset(&mut self, charset: TerminalCharset) {
        if self.charset != charset {
            self.charset = charset;
            self.decoder = None;
        }
    }

    pub fn decode(&mut self, input: &[u8]) -> String {
        let decoder = self
            .decoder
            .get_or_insert_with(|| self.charset.encoding().new_decoder());
        let capacity = self
            .charset
            .encoding()
            .max_utf8_buffer_length(input.len())
            .unwrap_or(input.len().saturating_mul(4).saturating_add(16));
        let mut output = String::with_capacity(capacity);
        let (_result, _read, _had_errors) = decoder.decode_to_string(input, &mut output, false);
        output
    }

    pub fn finish(&mut self) -> String {
        let Some(decoder) = self.decoder.as_mut() else {
            return String::new();
        };
        let mut output = String::with_capacity(16);
        let _ = decoder.decode_to_string(&[], &mut output, true);
        self.decoder = None;
        output
    }

    pub fn charset(&self) -> TerminalCharset {
        self.charset
    }
}

pub struct TerminalScreen {
    parser: Parser,
    decoder: TerminalDecoder,
    rows: u16,
    cols: u16,
}

impl TerminalScreen {
    pub fn new(rows: u16, cols: u16, scrollback_capacity: usize) -> Self {
        Self::with_charset(rows, cols, scrollback_capacity, TerminalCharset::Utf8)
    }

    pub fn with_charset(
        rows: u16,
        cols: u16,
        scrollback_capacity: usize,
        charset: TerminalCharset,
    ) -> Self {
        let rows = rows.max(1);
        let cols = cols.max(1);
        Self {
            parser: Parser::new(rows, cols, scrollback_capacity),
            decoder: TerminalDecoder::new(charset),
            rows,
            cols,
        }
    }

    pub fn set_charset(&mut self, charset: TerminalCharset) {
        self.decoder.set_charset(charset);
    }

    pub fn charset(&self) -> TerminalCharset {
        self.decoder.charset()
    }

    pub fn process(&mut self, bytes: &[u8]) {
        let decoded = self.decoder.decode(bytes);
        self.parser.process(decoded.as_bytes());
    }

    pub fn finish_input(&mut self) {
        let decoded = self.decoder.finish();
        self.parser.process(decoded.as_bytes());
    }

    pub fn resize(&mut self, rows: u16, cols: u16) {
        self.rows = rows.max(1);
        self.cols = cols.max(1);
        self.parser.screen_mut().set_size(self.rows, self.cols);
    }

    pub fn set_scrollback(&mut self, offset: usize) {
        self.parser.screen_mut().set_scrollback(offset);
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
        screen.process(b"\x1b[31mred\x1b[0m");
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
        screen.process(b"\x1b[?1049h");
        assert!(screen.alternate_screen());
        screen.process(b"\x1b[?1049l");
        assert!(!screen.alternate_screen());
    }

    #[test]
    fn gbk_multibyte_sequence_survives_packet_boundary() {
        let (bytes, _, _) = GBK.encode("中文");
        let bytes = bytes.into_owned();
        let mut decoder = TerminalDecoder::new(TerminalCharset::Gbk);
        let first = decoder.decode(&bytes[..1]);
        let second = decoder.decode(&bytes[1..]);
        assert_eq!(format!("{first}{second}"), "中文");
    }

    #[test]
    fn big5_decodes_traditional_chinese() {
        let (bytes, _, _) = BIG5.encode("繁體中文");
        let mut decoder = TerminalDecoder::new(TerminalCharset::Big5);
        assert_eq!(decoder.decode(bytes.as_ref()), "繁體中文");
    }

    #[test]
    fn changing_charset_resets_incomplete_sequence() {
        let mut decoder = TerminalDecoder::new(TerminalCharset::Gbk);
        let _ = decoder.decode(&[0xD6]);
        decoder.set_charset(TerminalCharset::Utf8);
        assert_eq!(decoder.decode("OK".as_bytes()), "OK");
    }
}
