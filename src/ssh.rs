use std::{path::PathBuf, sync::Arc, time::Duration};

use russh::{
    Channel, Disconnect,
    client::{self, Handler},
    keys::{self, PrivateKeyWithHashAlg, known_hosts},
};

use crate::config::{AUTH_AGENT, AUTH_KEY, AUTH_PASSWORD, Host};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostKeyPolicy {
    Strict,
    AcceptNew,
}

#[derive(Debug, Clone)]
pub struct SshOptions {
    pub known_hosts_path: PathBuf,
    pub host_key_policy: HostKeyPolicy,
    pub connect_timeout: Duration,
    pub keepalive_interval: Option<Duration>,
    pub keepalive_max: usize,
}

impl SshOptions {
    pub fn new(known_hosts_path: impl Into<PathBuf>) -> Self {
        Self {
            known_hosts_path: known_hosts_path.into(),
            host_key_policy: HostKeyPolicy::Strict,
            connect_timeout: Duration::from_secs(15),
            keepalive_interval: Some(Duration::from_secs(30)),
            keepalive_max: 3,
        }
    }
}

#[derive(Debug)]
pub enum SshCredentials {
    Password(String),
    PrivateKey {
        path: PathBuf,
        passphrase: Option<String>,
    },
}

#[derive(Debug)]
pub enum SshError {
    Timeout,
    UnknownHostKey { host: String, port: u16 },
    ChangedHostKey { host: String, port: u16 },
    AuthenticationRejected,
    MissingCredentials,
    UnsupportedAuthentication(String),
    KeyFile(keys::Error),
    KnownHosts(keys::Error),
    Transport(russh::Error),
}

impl std::fmt::Display for SshError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Timeout => write!(f, "SSH connection timed out"),
            Self::UnknownHostKey { host, port } => {
                write!(f, "unknown SSH host key for {host}:{port}")
            }
            Self::ChangedHostKey { host, port } => {
                write!(f, "SSH host key changed for {host}:{port}")
            }
            Self::AuthenticationRejected => write!(f, "SSH authentication rejected"),
            Self::MissingCredentials => write!(f, "SSH credentials are missing"),
            Self::UnsupportedAuthentication(kind) => {
                write!(f, "unsupported SSH authentication method: {kind}")
            }
            Self::KeyFile(error) => write!(f, "SSH private key: {error}"),
            Self::KnownHosts(error) => write!(f, "SSH known_hosts: {error}"),
            Self::Transport(error) => write!(f, "SSH transport: {error}"),
        }
    }
}

impl std::error::Error for SshError {}

impl From<russh::Error> for SshError {
    fn from(error: russh::Error) -> Self {
        Self::Transport(error)
    }
}

struct HostKeyVerifier {
    host: String,
    port: u16,
    path: PathBuf,
    policy: HostKeyPolicy,
}

impl Handler for HostKeyVerifier {
    type Error = SshError;

    async fn check_server_key(
        &mut self,
        server_public_key: &keys::PublicKey,
    ) -> Result<bool, Self::Error> {
        let known = known_hosts::known_host_keys_path(&self.host, self.port, &self.path)
            .map_err(SshError::KnownHosts)?;

        if known.iter().any(|(_, key)| key == server_public_key) {
            return Ok(true);
        }

        if !known.is_empty() {
            return Err(SshError::ChangedHostKey {
                host: self.host.clone(),
                port: self.port,
            });
        }

        match self.policy {
            HostKeyPolicy::Strict => Err(SshError::UnknownHostKey {
                host: self.host.clone(),
                port: self.port,
            }),
            HostKeyPolicy::AcceptNew => {
                known_hosts::learn_known_hosts_path(
                    &self.host,
                    self.port,
                    server_public_key,
                    &self.path,
                )
                .map_err(SshError::KnownHosts)?;
                Ok(true)
            }
        }
    }
}

pub struct SshClient {
    handle: client::Handle<HostKeyVerifier>,
    host: String,
    port: u16,
}

impl SshClient {
    pub async fn connect(
        host: &Host,
        credentials: SshCredentials,
        options: SshOptions,
    ) -> Result<Self, SshError> {
        let port = host.port_or_22();
        let verifier = HostKeyVerifier {
            host: host.addr.clone(),
            port,
            path: options.known_hosts_path,
            policy: options.host_key_policy,
        };
        let config = client::Config {
            keepalive_interval: options.keepalive_interval,
            keepalive_max: options.keepalive_max,
            nodelay: true,
            ..Default::default()
        };
        let connect = client::connect(Arc::new(config), (host.addr.as_str(), port), verifier);
        let mut handle = tokio::time::timeout(options.connect_timeout, connect)
            .await
            .map_err(|_| SshError::Timeout)??;

        let auth = match (host.auth.as_str(), credentials) {
            (AUTH_PASSWORD, SshCredentials::Password(password)) => {
                handle
                    .authenticate_password(host.user.clone(), password)
                    .await?
            }
            (AUTH_KEY, SshCredentials::PrivateKey { path, passphrase }) => {
                let key = keys::load_secret_key(expand_home(path), passphrase.as_deref())
                    .map_err(SshError::KeyFile)?;
                let hash = handle.best_supported_rsa_hash().await?.flatten();
                handle
                    .authenticate_publickey(
                        host.user.clone(),
                        PrivateKeyWithHashAlg::new(Arc::new(key), hash),
                    )
                    .await?
            }
            (AUTH_AGENT, _) => {
                return Err(SshError::UnsupportedAuthentication(AUTH_AGENT.to_owned()));
            }
            (kind, _) => {
                return Err(SshError::UnsupportedAuthentication(kind.to_owned()));
            }
        };

        if !auth.success() {
            return Err(SshError::AuthenticationRejected);
        }

        Ok(Self {
            handle,
            host: host.addr.clone(),
            port,
        })
    }

    pub fn is_closed(&self) -> bool {
        self.handle.is_closed()
    }

    pub fn target(&self) -> String {
        if self.port == 22 {
            self.host.clone()
        } else {
            format!("{}:{}", self.host, self.port)
        }
    }

    pub async fn open_shell(
        &self,
        term: &str,
        columns: u32,
        rows: u32,
    ) -> Result<Channel<client::Msg>, SshError> {
        let channel = self.handle.channel_open_session().await?;
        channel
            .request_pty(true, term, columns, rows, 0, 0, &[])
            .await?;
        channel.request_shell(true).await?;
        Ok(channel)
    }

    pub async fn resize_shell(
        channel: &Channel<client::Msg>,
        columns: u32,
        rows: u32,
    ) -> Result<(), SshError> {
        channel.window_change(columns, rows, 0, 0).await?;
        Ok(())
    }

    pub async fn send_input(
        channel: &Channel<client::Msg>,
        data: impl Into<Vec<u8>>,
    ) -> Result<(), SshError> {
        channel.data_bytes(data.into()).await?;
        Ok(())
    }

    pub async fn close_shell(channel: &Channel<client::Msg>) -> Result<(), SshError> {
        channel.eof().await?;
        channel.close().await?;
        Ok(())
    }

    pub async fn disconnect(self) -> Result<(), SshError> {
        self.handle
            .disconnect(
                Disconnect::ByApplication,
                "FinalShell closed the session",
                "en",
            )
            .await?;
        Ok(())
    }
}

fn expand_home(path: PathBuf) -> PathBuf {
    let text = path.to_string_lossy();
    if text == "~" || text.starts_with("~/") || text.starts_with("~\\") {
        let home = std::env::var_os("HOME")
            .or_else(|| std::env::var_os("USERPROFILE"))
            .map(PathBuf::from);
        if let Some(home) = home {
            return home.join(text[1..].trim_start_matches('/').trim_start_matches('\\'));
        }
    }
    path
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ssh_options_default_to_strict_host_key_verification() {
        let options = SshOptions::new("known_hosts");
        assert_eq!(options.host_key_policy, HostKeyPolicy::Strict);
        assert_eq!(options.connect_timeout, Duration::from_secs(15));
    }

    #[test]
    fn ssh_target_uses_the_effective_port() {
        let mut host = Host {
            addr: "example.com".into(),
            ..Host::default()
        };
        assert_eq!(host.port_or_22(), 22);
        host.port = 2222;
        assert_eq!(host.port_or_22(), 2222);
    }
}
