use std::sync::atomic::{AtomicU64, Ordering};

use russh::{Channel, ChannelMsg, client};

use crate::ssh::SshError;
use crate::terminal_input::{MouseEvent, TerminalInputEncoder, TerminalInputMode, TerminalKey};

static NEXT_TERMINAL_ID: AtomicU64 = AtomicU64::new(1);

pub struct TerminalSession {
    id: u64,
    channel: Channel<client::Msg>,
    input: TerminalInputEncoder,
    cols: u32,
    rows: u32,
    closed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TerminalEvent {
    Output { data: Vec<u8>, extended: bool },
    ExitStatus(u32),
    ExitSignal { signal: String, message: String },
    Eof,
    Closed,
}

impl TerminalSession {
    pub(crate) async fn open(
        channel: Channel<client::Msg>,
        term: &str,
        cols: u32,
        rows: u32,
    ) -> Result<Self, SshError> {
        let cols = cols.max(1);
        let rows = rows.max(1);
        channel
            .request_pty(true, term, cols, rows, 0, 0, &[])
            .await?;
        channel.request_shell(true).await?;
        Ok(Self {
            id: NEXT_TERMINAL_ID.fetch_add(1, Ordering::Relaxed),
            channel,
            input: TerminalInputEncoder::new(),
            cols,
            rows,
            closed: false,
        })
    }

    pub fn id(&self) -> u64 {
        self.id
    }

    pub fn size(&self) -> (u32, u32) {
        (self.cols, self.rows)
    }

    pub fn input_mode(&self) -> TerminalInputMode {
        self.input.mode()
    }

    pub async fn send(&self, data: impl Into<Vec<u8>>) -> Result<(), SshError> {
        if self.closed {
            return Err(SshError::TerminalClosed);
        }
        self.channel.data_bytes(data.into()).await?;
        Ok(())
    }

    pub async fn send_key(&self, key: TerminalKey) -> Result<(), SshError> {
        self.send(self.input.encode_key(key)).await
    }

    pub async fn paste(&self, text: &str) -> Result<(), SshError> {
        self.send(self.input.encode_paste(text)).await
    }

    pub async fn send_mouse(&self, event: MouseEvent) -> Result<bool, SshError> {
        let Some(data) = self.input.encode_mouse(event) else {
            return Ok(false);
        };
        self.send(data).await?;
        Ok(true)
    }

    pub async fn resize(&mut self, cols: u32, rows: u32) -> Result<(), SshError> {
        if self.closed {
            return Err(SshError::TerminalClosed);
        }
        let cols = cols.max(1);
        let rows = rows.max(1);
        if (cols, rows) == (self.cols, self.rows) {
            return Ok(());
        }
        self.channel.window_change(cols, rows, 0, 0).await?;
        self.cols = cols;
        self.rows = rows;
        Ok(())
    }

    pub async fn recv(&mut self) -> Result<Option<TerminalEvent>, SshError> {
        if self.closed {
            return Ok(None);
        }
        loop {
            match self.channel.wait().await {
                Some(ChannelMsg::Data { data }) => {
                    self.input.observe_output(&data);
                    return Ok(Some(TerminalEvent::Output {
                        data: data.to_vec(),
                        extended: false,
                    }));
                }
                Some(ChannelMsg::ExtendedData { data, .. }) => {
                    self.input.observe_output(&data);
                    return Ok(Some(TerminalEvent::Output {
                        data: data.to_vec(),
                        extended: true,
                    }));
                }
                Some(ChannelMsg::ExitStatus { exit_status }) => {
                    return Ok(Some(TerminalEvent::ExitStatus(exit_status)));
                }
                Some(ChannelMsg::ExitSignal {
                    signal_name,
                    error_message,
                    ..
                }) => {
                    return Ok(Some(TerminalEvent::ExitSignal {
                        signal: format!("{signal_name:?}"),
                        message: error_message,
                    }));
                }
                Some(ChannelMsg::Eof) => return Ok(Some(TerminalEvent::Eof)),
                Some(ChannelMsg::Close) | None => {
                    self.closed = true;
                    return Ok(Some(TerminalEvent::Closed));
                }
                Some(_) => continue,
            }
        }
    }

    pub async fn close(&mut self) -> Result<(), SshError> {
        if !self.closed {
            self.closed = true;
            self.channel.eof().await?;
            self.channel.close().await?;
        }
        Ok(())
    }

    pub fn is_closed(&self) -> bool {
        self.closed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_dimensions_are_never_zero() {
        assert_eq!((0_u32.max(1), 0_u32.max(1)), (1, 1));
    }

    #[test]
    fn terminal_ids_are_independent_and_monotonic() {
        let first = NEXT_TERMINAL_ID.fetch_add(1, Ordering::Relaxed);
        let second = NEXT_TERMINAL_ID.fetch_add(1, Ordering::Relaxed);
        assert!(second > first);
    }
}
