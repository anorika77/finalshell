#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MouseTracking {
    #[default]
    Off,
    X10,
    Normal,
    ButtonEvent,
    AnyEvent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TerminalInputMode {
    pub application_cursor: bool,
    pub application_keypad: bool,
    pub bracketed_paste: bool,
    pub mouse_tracking: MouseTracking,
    pub sgr_mouse: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalKey {
    Up, Down, Right, Left, Home, End, Insert, Delete, PageUp, PageDown,
    F(u8), Enter, Tab, Backspace, Escape, Ctrl(char),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MouseEvent {
    pub button: u8,
    pub x: u16,
    pub y: u16,
    pub pressed: bool,
    pub shift: bool,
    pub alt: bool,
    pub ctrl: bool,
    pub motion: bool,
}

#[derive(Debug, Default)]
pub struct TerminalInputEncoder {
    mode: TerminalInputMode,
    csi: Vec<u8>,
    in_csi: bool,
    saw_escape: bool,
}

impl TerminalInputEncoder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn mode(&self) -> TerminalInputMode {
        self.mode
    }

    /// Track DEC input modes in server output, including sequences split across packets.
    pub fn observe_output(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            if self.in_csi {
                if (0x40..=0x7e).contains(&byte) {
                    self.csi.push(byte);
                    self.apply_csi();
                    self.csi.clear();
                    self.in_csi = false;
                } else if (0x20..=0x3f).contains(&byte) && self.csi.len() < 128 {
                    self.csi.push(byte);
                } else {
                    self.csi.clear();
                    self.in_csi = false;
                }
                continue;
            }

            if self.saw_escape {
                self.saw_escape = false;
                match byte {
                    b'[' => {
                        self.in_csi = true;
                        self.csi.clear();
                    }
                    b'=' => self.mode.application_keypad = true,
                    b'>' => self.mode.application_keypad = false,
                    _ => {}
                }
                continue;
            }

            if byte == 0x1b {
                self.saw_escape = true;
            }
        }
    }

    fn apply_csi(&mut self) {
        let Some((&final_byte, body)) = self.csi.split_last() else {
            return;
        };
        if final_byte != b'h' && final_byte != b'l' {
            return;
        }
        let Ok(body) = std::str::from_utf8(body) else {
            return;
        };
        let Some(params) = body.strip_prefix('?') else {
            return;
        };
        let enabled = final_byte == b'h';
        for value in params.split(';').filter_map(|part| part.parse::<u16>().ok()) {
            match value {
                1 => self.mode.application_cursor = enabled,
                1000 => {
                    self.mode.mouse_tracking = if enabled {
                        MouseTracking::Normal
                    } else {
                        MouseTracking::Off
                    };
                }
                1002 => {
                    self.mode.mouse_tracking = if enabled {
                        MouseTracking::ButtonEvent
                    } else {
                        MouseTracking::Off
                    };
                }
                1003 => {
                    self.mode.mouse_tracking = if enabled {
                        MouseTracking::AnyEvent
                    } else {
                        MouseTracking::Off
                    };
                }
                1006 => self.mode.sgr_mouse = enabled,
                2004 => self.mode.bracketed_paste = enabled,
                _ => {}
            }
        }
    }

    pub fn encode_key(&self, key: TerminalKey) -> Vec<u8> {
        let bytes: &[u8] = match key {
            TerminalKey::Up if self.mode.application_cursor => b"\x1bOA",
            TerminalKey::Down if self.mode.application_cursor => b"\x1bOB",
            TerminalKey::Right if self.mode.application_cursor => b"\x1bOC",
            TerminalKey::Left if self.mode.application_cursor => b"\x1bOD",
            TerminalKey::Up => b"\x1b[A",
            TerminalKey::Down => b"\x1b[B",
            TerminalKey::Right => b"\x1b[C",
            TerminalKey::Left => b"\x1b[D",
            TerminalKey::Home => {
                if self.mode.application_cursor { b"\x1bOH" } else { b"\x1b[H" }
            }
            TerminalKey::End => {
                if self.mode.application_cursor { b"\x1bOF" } else { b"\x1b[F" }
            }
            TerminalKey::Insert => b"\x1b[2~",
            TerminalKey::Delete => b"\x1b[3~",
            TerminalKey::PageUp => b"\x1b[5~",
            TerminalKey::PageDown => b"\x1b[6~",
            TerminalKey::F(1) => b"\x1bOP",
            TerminalKey::F(2) => b"\x1bOQ",
            TerminalKey::F(3) => b"\x1bOR",
            TerminalKey::F(4) => b"\x1bOS",
            TerminalKey::F(5) => b"\x1b[15~",
            TerminalKey::F(6) => b"\x1b[17~",
            TerminalKey::F(7) => b"\x1b[18~",
            TerminalKey::F(8) => b"\x1b[19~",
            TerminalKey::F(9) => b"\x1b[20~",
            TerminalKey::F(10) => b"\x1b[21~",
            TerminalKey::F(11) => b"\x1b[23~",
            TerminalKey::F(12) => b"\x1b[24~",
            TerminalKey::Enter => b"\r",
            TerminalKey::Tab => b"\t",
            TerminalKey::Backspace => b"\x7f",
            TerminalKey::Escape => b"\x1b",
            TerminalKey::Ctrl(ch) if ch.is_ascii_alphabetic() => {
                return vec![ch.to_ascii_uppercase() as u8 - b'A' + 1];
            }
            TerminalKey::Ctrl('@') => b"\0",
            TerminalKey::Ctrl('[') => b"\x1b",
            TerminalKey::Ctrl('\\') => b"\x1c",
            TerminalKey::Ctrl(']') => b"\x1d",
            TerminalKey::Ctrl('^') => b"\x1e",
            TerminalKey::Ctrl('_') => b"\x1f",
            TerminalKey::Ctrl('?') => b"\x7f",
            TerminalKey::F(_) => return Vec::new(),
        };
        bytes.to_vec()
    }

    pub fn encode_paste(&self, text: &str) -> Vec<u8> {
        let mut output = Vec::with_capacity(text.len() + 16);
        if self.mode.bracketed_paste {
            output.extend_from_slice(b"\x1b[200~");
        }
        output.extend_from_slice(text.as_bytes());
        if self.mode.bracketed_paste {
            output.extend_from_slice(b"\x1b[201~");
        }
        output
    }

    pub fn encode_mouse(&self, event: MouseEvent) -> Option<Vec<u8>> {
        if self.mode.mouse_tracking == MouseTracking::Off {
            return None;
        }
        if event.motion && self.mode.mouse_tracking == MouseTracking::Normal {
            return None;
        }
        if event.motion
            && self.mode.mouse_tracking == MouseTracking::ButtonEvent
            && event.button == 3
        {
            return None;
        }

        let mut code = event.button.min(2) as u16;
        if event.shift {
            code += 4;
        }
        if event.alt {
            code += 8;
        }
        if event.ctrl {
            code += 16;
        }
        if event.motion {
            code += 32;
        }

        let x = event.x.max(1);
        let y = event.y.max(1);
        if self.mode.sgr_mouse {
            let suffix = if event.pressed { 'M' } else { 'm' };
            Some(format!("\x1b[<{code};{x};{y}{suffix}").into_bytes())
        } else if x <= 223 && y <= 223 {
            let button = if event.pressed { code as u8 } else { 3 };
            Some(vec![
                0x1b,
                b'[',
                b'M',
                32 + button,
                32 + x as u8,
                32 + y as u8,
            ])
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_keys_follow_application_mode_across_packet_boundary() {
        let mut encoder = TerminalInputEncoder::new();
        encoder.observe_output(b"\x1b[?");
        encoder.observe_output(b"1h");
        assert_eq!(encoder.encode_key(TerminalKey::Up), b"\x1bOA");
        encoder.observe_output(b"\x1b[?1l");
        assert_eq!(encoder.encode_key(TerminalKey::Up), b"\x1b[A");
    }

    #[test]
    fn function_and_navigation_keys_are_encoded() {
        let encoder = TerminalInputEncoder::new();
        assert_eq!(encoder.encode_key(TerminalKey::F(12)), b"\x1b[24~");
        assert_eq!(encoder.encode_key(TerminalKey::PageDown), b"\x1b[6~");
        assert_eq!(encoder.encode_key(TerminalKey::Ctrl('c')), vec![3]);
    }

    #[test]
    fn bracketed_paste_tracks_mode() {
        let mut encoder = TerminalInputEncoder::new();
        assert_eq!(encoder.encode_paste("abc"), b"abc");
        encoder.observe_output(b"\x1b[?2004h");
        assert_eq!(encoder.encode_paste("abc"), b"\x1b[200~abc\x1b[201~");
    }

    #[test]
    fn mouse_tracking_and_sgr_protocol() {
        let mut encoder = TerminalInputEncoder::new();
        encoder.observe_output(b"\x1b[?1000;1006h");
        let event = MouseEvent {
            button: 0,
            x: 12,
            y: 7,
            pressed: true,
            shift: false,
            alt: false,
            ctrl: false,
            motion: false,
        };
        assert_eq!(encoder.encode_mouse(event).unwrap(), b"\x1b[<0;12;7M");
    }

    #[test]
    fn keypad_application_mode_is_tracked() {
        let mut encoder = TerminalInputEncoder::new();
        encoder.observe_output(b"\x1b=");
        assert!(encoder.mode().application_keypad);
        encoder.observe_output(b"\x1b>");
        assert!(!encoder.mode().application_keypad);
    }
}
