use std::{collections::HashMap, path::PathBuf, sync::Arc, time::Duration};

use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};

use russh::{
    Channel, Disconnect,
    client::{self, Handler},
    keys::{
        self, PrivateKeyWithHashAlg,
        agent::{AgentIdentity, client::AgentClient},
        known_hosts,
    },
};

use crate::config::{AUTH_AGENT, AUTH_KEY, AUTH_KEYBOARD_INTERACTIVE, AUTH_PASSWORD, Host};

pub(crate) type RemoteForwardRoutes =
    Arc<tokio::sync::Mutex<HashMap<u16, tokio::sync::mpsc::UnboundedSender<Channel<client::Msg>>>>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostKeyPolicy {
    Strict,
    Ask,
    AcceptNew,
}

#[derive(Debug)]
pub struct HostKeyPrompt {
    pub host: String,
    pub port: u16,
    pub key_type: String,
    pub fingerprint: String,
    pub reply: tokio::sync::oneshot::Sender<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyboardInteractivePrompt {
    pub prompt: String,
    pub echo: bool,
}

#[derive(Debug)]
pub struct KeyboardInteractiveChallenge {
    pub name: String,
    pub instructions: String,
    pub prompts: Vec<KeyboardInteractivePrompt>,
    pub reply: tokio::sync::oneshot::Sender<Vec<String>>,
}

#[derive(Debug, Clone)]
pub struct SshOptions {
    pub known_hosts_path: PathBuf,
    pub host_key_policy: HostKeyPolicy,
    pub connect_timeout: Duration,
    pub keepalive_interval: Option<Duration>,
    pub keepalive_max: usize,
    pub host_key_prompt: Option<tokio::sync::mpsc::Sender<HostKeyPrompt>>,
    pub host_key_prompt_timeout: Duration,
    pub keyboard_interactive_prompt:
        Option<tokio::sync::mpsc::Sender<KeyboardInteractiveChallenge>>,
    pub keyboard_interactive_timeout: Duration,
    pub proxy_password: Option<String>,
    pub jump_host: Option<Box<JumpHostConfig>>,
}

#[derive(Debug, Clone)]
pub struct JumpHostConfig {
    pub host: Host,
    pub credentials: SshCredentials,
    pub proxy_password: Option<String>,
    pub jump: Option<Box<JumpHostConfig>>,
}

impl SshOptions {
    pub fn new(known_hosts_path: impl Into<PathBuf>) -> Self {
        Self {
            known_hosts_path: known_hosts_path.into(),
            host_key_policy: HostKeyPolicy::Strict,
            connect_timeout: Duration::from_secs(15),
            keepalive_interval: Some(Duration::from_secs(30)),
            keepalive_max: 3,
            host_key_prompt: None,
            host_key_prompt_timeout: Duration::from_secs(120),
            keyboard_interactive_prompt: None,
            keyboard_interactive_timeout: Duration::from_secs(120),
            proxy_password: None,
            jump_host: None,
        }
    }
}

#[derive(Debug, Clone)]
pub enum SshCredentials {
    Password(String),
    PrivateKey {
        path: PathBuf,
        passphrase: Option<String>,
    },
    KeyboardInteractive,
}

#[derive(Debug)]
pub enum SshError {
    Timeout,
    Proxy(String),
    Sftp(String),
    UnknownHostKey { host: String, port: u16 },
    ChangedHostKey { host: String, port: u16 },
    AuthenticationRejected,
    HostKeyConfirmationRejected,
    HostKeyConfirmationUnavailable,
    HostKeyConfirmationTimeout,
    KeyboardInteractiveUnavailable,
    KeyboardInteractiveTimeout,
    KeyboardInteractiveResponseCount { expected: usize, actual: usize },
    MissingCredentials,
    UnsupportedAuthentication(String),
    KeyFile(keys::Error),
    Agent(keys::Error),
    KnownHosts(keys::Error),
    Transport(russh::Error),
}

impl std::fmt::Display for SshError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Timeout => write!(f, "SSH connection timed out"),
            Self::Proxy(message) => write!(f, "SSH proxy: {message}"),
            Self::Sftp(message) => write!(f, "SSH SFTP: {message}"),
            Self::UnknownHostKey { host, port } => {
                write!(f, "unknown SSH host key for {host}:{port}")
            }
            Self::ChangedHostKey { host, port } => {
                write!(f, "SSH host key changed for {host}:{port}")
            }
            Self::AuthenticationRejected => write!(f, "SSH authentication rejected"),
            Self::HostKeyConfirmationRejected => write!(f, "SSH host key was rejected by the user"),
            Self::HostKeyConfirmationUnavailable => {
                write!(f, "SSH host key confirmation UI is unavailable")
            }
            Self::HostKeyConfirmationTimeout => write!(f, "SSH host key confirmation timed out"),
            Self::KeyboardInteractiveUnavailable => {
                write!(f, "SSH keyboard-interactive prompt UI is unavailable")
            }
            Self::KeyboardInteractiveTimeout => {
                write!(f, "SSH keyboard-interactive response timed out")
            }
            Self::KeyboardInteractiveResponseCount { expected, actual } => write!(
                f,
                "SSH keyboard-interactive expected {expected} responses, received {actual}"
            ),
            Self::MissingCredentials => write!(f, "SSH credentials are missing"),
            Self::UnsupportedAuthentication(kind) => {
                write!(f, "unsupported SSH authentication method: {kind}")
            }
            Self::KeyFile(error) => write!(f, "SSH private key: {error}"),
            Self::Agent(error) => write!(f, "SSH agent: {error}"),
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

pub(crate) struct HostKeyVerifier {
    host: String,
    port: u16,
    path: PathBuf,
    policy: HostKeyPolicy,
    prompt: Option<tokio::sync::mpsc::Sender<HostKeyPrompt>>,
    prompt_timeout: Duration,
    remote_forward_routes: RemoteForwardRoutes,
}

impl Handler for HostKeyVerifier {
    type Error = SshError;

    async fn check_server_key(
        &mut self,
        server_public_key: &keys::PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        let server_public_key = server_public_key.public_key();
        let known = known_hosts::known_host_keys_path(&self.host, self.port, &self.path)
            .map_err(SshError::KnownHosts)?;

        if known.iter().any(|(_, key)| key == &server_public_key) {
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
            HostKeyPolicy::Ask => {
                let Some(sender) = &self.prompt else {
                    return Err(SshError::UnknownHostKey {
                        host: self.host.clone(),
                        port: self.port,
                    });
                };
                let (reply, response) = tokio::sync::oneshot::channel();
                let prompt = HostKeyPrompt {
                    host: self.host.clone(),
                    port: self.port,
                    key_type: server_public_key.algorithm().as_str().to_owned(),
                    fingerprint: server_public_key
                        .fingerprint(keys::HashAlg::Sha256)
                        .to_string(),
                    reply,
                };
                sender
                    .send(prompt)
                    .await
                    .map_err(|_| SshError::HostKeyConfirmationUnavailable)?;
                let accepted = tokio::time::timeout(self.prompt_timeout, response)
                    .await
                    .map_err(|_| SshError::HostKeyConfirmationTimeout)?
                    .map_err(|_| SshError::HostKeyConfirmationUnavailable)?;
                if !accepted {
                    return Err(SshError::HostKeyConfirmationRejected);
                }
                known_hosts::learn_known_hosts_path(
                    &self.host,
                    self.port,
                    &server_public_key,
                    &self.path,
                )
                .map_err(SshError::KnownHosts)?;
                Ok(true)
            }
            HostKeyPolicy::AcceptNew => {
                known_hosts::learn_known_hosts_path(
                    &self.host,
                    self.port,
                    &server_public_key,
                    &self.path,
                )
                .map_err(SshError::KnownHosts)?;
                Ok(true)
            }
        }
    }

    async fn server_channel_open_forwarded_tcpip(
        &mut self,
        channel: Channel<client::Msg>,
        _connected_address: &str,
        connected_port: u32,
        _originator_address: &str,
        _originator_port: u32,
        reply: client::ChannelOpenHandle,
        _session: &mut client::Session,
    ) -> Result<(), Self::Error> {
        let sender = {
            let routes = self.remote_forward_routes.lock().await;
            u16::try_from(connected_port)
                .ok()
                .and_then(|port| routes.get(&port).or_else(|| routes.get(&0)).cloned())
        };
        if let Some(sender) = sender {
            reply.accept().await;
            sender.send(channel).map_err(|_| {
                SshError::Proxy("remote forwarding receiver has been closed".into())
            })?;
        }
        Ok(())
    }
}

struct ManagedTunnel {
    spec: crate::config::Tunnel,
    handle: Option<crate::forward::ForwardHandle>,
    error: Option<String>,
}

pub struct SshClient {
    handle: Arc<client::Handle<HostKeyVerifier>>,
    host: String,
    port: u16,
    jump_chain: Vec<SshClient>,
    remote_forward_routes: RemoteForwardRoutes,
    tunnels: tokio::sync::Mutex<Vec<ManagedTunnel>>,
}

trait SshTransport: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send {}
impl<T> SshTransport for T where T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send {}

impl SshClient {
    pub async fn connect(
        host: &Host,
        credentials: SshCredentials,
        options: SshOptions,
    ) -> Result<Self, SshError> {
        let mut client =
            Self::connect_inner(host.clone(), credentials, options, Vec::new()).await?;
        client.start_configured_tunnels(&host.tunnels).await;
        Ok(client)
    }

    fn connect_inner(
        host: Host,
        credentials: SshCredentials,
        options: SshOptions,
        mut visited: Vec<String>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Self, SshError>> + Send>> {
        Box::pin(async move {
            if visited.len() >= 8 {
                return Err(SshError::Proxy(
                    "Jump Host chain exceeds the maximum depth of 8".into(),
                ));
            }
            if !host.id.is_empty() {
                if visited.iter().any(|id| id == &host.id) {
                    return Err(SshError::Proxy(format!(
                        "Jump Host cycle detected at host {}",
                        host.id
                    )));
                }
                visited.push(host.id.clone());
            }

            let port = host.port_or_22();
            let remote_forward_routes = Arc::new(tokio::sync::Mutex::new(HashMap::new()));
            let verifier = HostKeyVerifier {
                host: host.addr.clone(),
                port,
                path: options.known_hosts_path.clone(),
                policy: options.host_key_policy,
                prompt: options.host_key_prompt.clone(),
                prompt_timeout: options.host_key_prompt_timeout,
                remote_forward_routes: remote_forward_routes.clone(),
            };
            let config = client::Config {
                keepalive_interval: options.keepalive_interval,
                keepalive_max: options.keepalive_max,
                nodelay: true,
                ..Default::default()
            };
            let mut jump_chain = Vec::new();
            let connect = async {
                match host.proxy.as_ref().filter(|proxy| !proxy.kind.is_empty()) {
                    None => {
                        client::connect(Arc::new(config), (host.addr.as_str(), port), verifier)
                            .await
                    }
                    Some(proxy) => {
                        let stream: Box<dyn SshTransport> = match proxy.kind.as_str() {
                            "socks5" => Box::new(
                                dial_socks5(
                                    proxy,
                                    options.proxy_password.as_deref(),
                                    &host.addr,
                                    port,
                                )
                                .await?,
                            ),
                            "http" => Box::new(
                                dial_http_connect(
                                    proxy,
                                    options.proxy_password.as_deref(),
                                    &host.addr,
                                    port,
                                )
                                .await?,
                            ),
                            "jump" => {
                                let jump = options.jump_host.as_ref().ok_or_else(|| {
                                    SshError::Proxy(format!(
                                        "host {} requires Jump Host {}, but no jump credentials were supplied",
                                        host.name,
                                        proxy.jump_host
                                    ))
                                })?;
                                if !proxy.jump_host.is_empty() && jump.host.id != proxy.jump_host {
                                    return Err(SshError::Proxy(format!(
                                        "configured Jump Host ID {} does not match required ID {}",
                                        jump.host.id, proxy.jump_host
                                    )));
                                }
                                let mut jump_options = options.clone();
                                jump_options.proxy_password = jump.proxy_password.clone();
                                jump_options.jump_host = jump.jump.clone();
                                let jump_client = Self::connect_inner(
                                    jump.host.clone(),
                                    jump.credentials.clone(),
                                    jump_options,
                                    visited.clone(),
                                )
                                .await?;
                                let jump_channel = tokio::time::timeout(
                                    options.connect_timeout,
                                    jump_client.handle.channel_open_direct_tcpip(
                                        host.addr.clone(),
                                        port as u32,
                                        "127.0.0.1",
                                        0,
                                    ),
                                )
                                .await
                                .map_err(|_| SshError::Timeout)??;
                                jump_chain.push(jump_client);
                                Box::new(jump_channel.into_stream())
                            }
                            kind => {
                                return Err(SshError::Proxy(format!(
                                    "unsupported proxy type: {kind}"
                                )));
                            }
                        };
                        client::connect_stream(Arc::new(config), stream, verifier).await
                    }
                }
            };
            let mut handle = tokio::time::timeout(options.connect_timeout, connect)
                .await
                .map_err(|_| SshError::Timeout)??;

            let authenticated = match (host.auth.as_str(), credentials) {
                (AUTH_PASSWORD, SshCredentials::Password(password)) => handle
                    .authenticate_password(host.user.clone(), password)
                    .await?
                    .success(),
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
                        .success()
                }
                (AUTH_AGENT, _) => authenticate_agent(&mut handle, &host.user).await?.success(),
                (AUTH_KEYBOARD_INTERACTIVE, SshCredentials::KeyboardInteractive) => {
                    authenticate_keyboard_interactive(
                        &mut handle,
                        &host.user,
                        options.keyboard_interactive_prompt,
                        options.keyboard_interactive_timeout,
                    )
                    .await?
                }
                (kind, _) => {
                    return Err(SshError::UnsupportedAuthentication(kind.to_owned()));
                }
            };

            if !authenticated {
                return Err(SshError::AuthenticationRejected);
            }

            Ok(Self {
                handle: Arc::new(handle),
                host: host.addr.clone(),
                port,
                jump_chain,
                remote_forward_routes,
                tunnels: tokio::sync::Mutex::new(Vec::new()),
            })
        })
    }

    async fn start_configured_tunnels(&mut self, specs: &[crate::config::Tunnel]) {
        let mut runs = Vec::with_capacity(specs.len());
        for spec in specs {
            let mut run = ManagedTunnel {
                spec: spec.clone(),
                handle: None,
                error: None,
            };
            if !spec.disabled {
                match crate::forward::start_configured_tunnel(
                    self.forwarding_handle(),
                    self.remote_forward_routes.clone(),
                    spec,
                )
                .await
                {
                    Ok(handle) => run.handle = Some(handle),
                    Err(error) => run.error = Some(error.to_string()),
                }
            }
            runs.push(run);
        }
        *self.tunnels.get_mut() = runs;
    }

    /// Returns the configured tunnel states in their original host-config order.
    pub async fn tunnel_statuses(&self) -> Vec<crate::forward::TunnelStatus> {
        self.tunnels
            .lock()
            .await
            .iter()
            .map(|run| crate::forward::TunnelStatus {
                spec: run.spec.clone(),
                listening: run.handle.is_some(),
                address: run.handle.as_ref().map(|handle| handle.local_addr()),
                error: run.error.clone(),
                stats: run
                    .handle
                    .as_ref()
                    .map(crate::forward::ForwardHandle::stats)
                    .unwrap_or_default(),
            })
            .collect()
    }

    /// Starts one configured tunnel by its zero-based position in Host.tunnels.
    pub async fn start_tunnel(&self, index: usize) -> Result<(), SshError> {
        let handle = self.forwarding_handle();
        let routes = self.remote_forward_routes.clone();
        let mut runs = self.tunnels.lock().await;
        let run = runs
            .get_mut(index)
            .ok_or_else(|| SshError::Proxy(format!("tunnel index {index} does not exist")))?;
        if run.handle.is_some() {
            return Ok(());
        }
        match crate::forward::start_configured_tunnel(handle, routes, &run.spec).await {
            Ok(handle) => {
                run.handle = Some(handle);
                run.error = None;
                Ok(())
            }
            Err(error) => {
                run.error = Some(error.to_string());
                Err(error)
            }
        }
    }

    /// Stops one tunnel but keeps its configuration and status row.
    pub async fn stop_tunnel(&self, index: usize) -> Result<(), SshError> {
        let handle = {
            let mut runs = self.tunnels.lock().await;
            let run = runs
                .get_mut(index)
                .ok_or_else(|| SshError::Proxy(format!("tunnel index {index} does not exist")))?;
            run.error = None;
            run.handle.take()
        };
        if let Some(handle) = handle {
            handle.close().await;
        }
        Ok(())
    }

    /// Stops all tunnels before the SSH transport is disconnected.
    pub async fn stop_tunnels(&self) {
        let handles = {
            let mut runs = self.tunnels.lock().await;
            runs.iter_mut()
                .filter_map(|run| run.handle.take())
                .collect::<Vec<_>>()
        };
        for handle in handles {
            handle.close().await;
        }
    }

    pub(crate) fn forwarding_handle(&self) -> Arc<client::Handle<HostKeyVerifier>> {
        self.handle.clone()
    }

    pub async fn start_forward(
        &self,
        spec: crate::forward::ForwardSpec,
    ) -> Result<crate::forward::ForwardHandle, SshError> {
        crate::forward::start(self.forwarding_handle(), spec).await
    }

    pub async fn start_remote_forward(
        &self,
        spec: crate::forward::RemoteForwardSpec,
    ) -> Result<crate::forward::ForwardHandle, SshError> {
        crate::forward::start_remote(
            self.handle.clone(),
            self.remote_forward_routes.clone(),
            spec,
        )
        .await
    }

    pub async fn open_sftp(&self) -> Result<russh_sftp::client::SftpSession, SshError> {
        let channel = self.handle.channel_open_session().await?;
        channel.request_subsystem(true, "sftp").await?;
        russh_sftp::client::SftpSession::new(channel.into_stream())
            .await
            .map_err(|error| SshError::Sftp(error.to_string()))
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
        let data = data.into();
        channel.data(data.as_slice()).await?;
        Ok(())
    }

    pub async fn close_shell(channel: &Channel<client::Msg>) -> Result<(), SshError> {
        channel.eof().await?;
        channel.close().await?;
        Ok(())
    }

    pub async fn disconnect(mut self) -> Result<(), SshError> {
        self.stop_tunnels().await;
        let mut result = self
            .handle
            .disconnect(
                Disconnect::ByApplication,
                "FinalShell closed the session",
                "en",
            )
            .await
            .map_err(SshError::from);
        for jump in self.jump_chain.drain(..).rev() {
            if let Err(error) = Box::pin(jump.disconnect()).await
                && result.is_ok()
            {
                result = Err(error);
            }
        }
        result
    }
}

async fn dial_socks5(
    proxy: &crate::config::ProxyConfig,
    password: Option<&str>,
    target_host: &str,
    target_port: u16,
) -> Result<TcpStream, SshError> {
    let port = if proxy.port == 0 { 1080 } else { proxy.port };
    let addr = format!("{}:{port}", proxy.addr);
    let mut stream = TcpStream::connect(&addr)
        .await
        .map_err(|error| SshError::Proxy(format!("cannot connect to {addr}: {error}")))?;
    let authenticated = !proxy.user.is_empty();
    stream
        .write_all(if authenticated {
            &[5, 2, 0, 2]
        } else {
            &[5, 1, 0]
        })
        .await
        .map_err(|error| SshError::Proxy(error.to_string()))?;
    let mut method = [0u8; 2];
    stream
        .read_exact(&mut method)
        .await
        .map_err(|error| SshError::Proxy(error.to_string()))?;
    if method[0] != 5 || method[1] == 0xff || (method[1] == 2 && !authenticated) {
        return Err(SshError::Proxy(
            "SOCKS5 authentication method rejected".into(),
        ));
    }
    if method[1] == 2 {
        let user = proxy.user.as_bytes();
        let pass = password.unwrap_or_default().as_bytes();
        if user.len() > 255 || pass.len() > 255 {
            return Err(SshError::Proxy(
                "SOCKS5 credentials exceed 255 bytes".into(),
            ));
        }
        let mut auth = Vec::with_capacity(user.len() + pass.len() + 3);
        auth.extend_from_slice(&[1, user.len() as u8]);
        auth.extend_from_slice(user);
        auth.push(pass.len() as u8);
        auth.extend_from_slice(pass);
        stream
            .write_all(&auth)
            .await
            .map_err(|error| SshError::Proxy(error.to_string()))?;
        let mut result = [0u8; 2];
        stream
            .read_exact(&mut result)
            .await
            .map_err(|error| SshError::Proxy(error.to_string()))?;
        if result != [1, 0] {
            return Err(SshError::Proxy(
                "SOCKS5 username/password authentication failed".into(),
            ));
        }
    } else if method[1] != 0 {
        return Err(SshError::Proxy(format!(
            "unsupported SOCKS5 auth method {}",
            method[1]
        )));
    }
    let host = target_host.as_bytes();
    if host.is_empty() || host.len() > 255 {
        return Err(SshError::Proxy(
            "SOCKS5 target hostname must be 1..255 bytes".into(),
        ));
    }
    let mut request = Vec::with_capacity(host.len() + 7);
    request.extend_from_slice(&[5, 1, 0, 3, host.len() as u8]);
    request.extend_from_slice(host);
    request.extend_from_slice(&target_port.to_be_bytes());
    stream
        .write_all(&request)
        .await
        .map_err(|error| SshError::Proxy(error.to_string()))?;
    let mut head = [0u8; 4];
    stream
        .read_exact(&mut head)
        .await
        .map_err(|error| SshError::Proxy(error.to_string()))?;
    if head[0] != 5 || head[1] != 0 {
        return Err(SshError::Proxy(format!(
            "SOCKS5 CONNECT failed with reply {}",
            head[1]
        )));
    }
    let address_len = match head[3] {
        1 => 4,
        4 => 16,
        3 => {
            let mut len = [0u8; 1];
            stream
                .read_exact(&mut len)
                .await
                .map_err(|error| SshError::Proxy(error.to_string()))?;
            len[0] as usize
        }
        kind => {
            return Err(SshError::Proxy(format!(
                "invalid SOCKS5 address type {kind}"
            )));
        }
    };
    let mut discard = vec![0u8; address_len + 2];
    stream
        .read_exact(&mut discard)
        .await
        .map_err(|error| SshError::Proxy(error.to_string()))?;
    Ok(stream)
}

async fn dial_http_connect(
    proxy: &crate::config::ProxyConfig,
    password: Option<&str>,
    target_host: &str,
    target_port: u16,
) -> Result<TcpStream, SshError> {
    let port = if proxy.port == 0 { 8080 } else { proxy.port };
    let addr = format!("{}:{port}", proxy.addr);
    let mut stream = TcpStream::connect(&addr)
        .await
        .map_err(|error| SshError::Proxy(format!("cannot connect to {addr}: {error}")))?;
    let target = format!("{target_host}:{target_port}");
    let mut request = format!("CONNECT {target} HTTP/1.1\r\nHost: {target}\r\n");
    if !proxy.user.is_empty() {
        let token =
            base64_encode(format!("{}:{}", proxy.user, password.unwrap_or_default()).as_bytes());
        request.push_str(&format!("Proxy-Authorization: Basic {token}\r\n"));
    }
    request.push_str("\r\n");
    stream
        .write_all(request.as_bytes())
        .await
        .map_err(|error| SshError::Proxy(error.to_string()))?;
    let mut response = Vec::with_capacity(512);
    let mut byte = [0u8; 1];
    while response.len() < 16 * 1024 {
        stream
            .read_exact(&mut byte)
            .await
            .map_err(|error| SshError::Proxy(error.to_string()))?;
        response.push(byte[0]);
        if response.ends_with(b"\r\n\r\n") {
            break;
        }
    }
    if !response.ends_with(b"\r\n\r\n") {
        return Err(SshError::Proxy(
            "HTTP CONNECT response headers too large".into(),
        ));
    }
    let headers = String::from_utf8_lossy(&response);
    let status = headers.lines().next().unwrap_or_default();
    let code = status
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse::<u16>().ok())
        .ok_or_else(|| SshError::Proxy("invalid HTTP CONNECT response".into()))?;
    if !(200..300).contains(&code) {
        return Err(SshError::Proxy(format!("HTTP CONNECT rejected: {status}")));
    }
    Ok(stream)
}

fn base64_encode(input: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut output = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let a = chunk[0];
        let b = *chunk.get(1).unwrap_or(&0);
        let c = *chunk.get(2).unwrap_or(&0);
        output.push(TABLE[(a >> 2) as usize] as char);
        output.push(TABLE[(((a & 3) << 4) | (b >> 4)) as usize] as char);
        output.push(if chunk.len() > 1 {
            TABLE[(((b & 15) << 2) | (c >> 6)) as usize] as char
        } else {
            '='
        });
        output.push(if chunk.len() > 2 {
            TABLE[(c & 63) as usize] as char
        } else {
            '='
        });
    }
    output
}

async fn authenticate_keyboard_interactive(
    handle: &mut client::Handle<HostKeyVerifier>,
    user: &str,
    prompt_sender: Option<tokio::sync::mpsc::Sender<KeyboardInteractiveChallenge>>,
    timeout: Duration,
) -> Result<bool, SshError> {
    use client::KeyboardInteractiveAuthResponse;

    let mut response = tokio::time::timeout(
        timeout,
        handle.authenticate_keyboard_interactive_start(user.to_owned(), None),
    )
    .await
    .map_err(|_| SshError::KeyboardInteractiveTimeout)??;

    loop {
        match response {
            KeyboardInteractiveAuthResponse::Success => return Ok(true),
            KeyboardInteractiveAuthResponse::Failure { .. } => return Ok(false),
            KeyboardInteractiveAuthResponse::InfoRequest {
                name,
                instructions,
                prompts,
            } => {
                let Some(sender) = &prompt_sender else {
                    return Err(SshError::KeyboardInteractiveUnavailable);
                };
                let expected = prompts.len();
                let (reply, answer) = tokio::sync::oneshot::channel();
                let challenge = KeyboardInteractiveChallenge {
                    name,
                    instructions,
                    prompts: prompts
                        .into_iter()
                        .map(|prompt| KeyboardInteractivePrompt {
                            prompt: prompt.prompt,
                            echo: prompt.echo,
                        })
                        .collect(),
                    reply,
                };
                sender
                    .send(challenge)
                    .await
                    .map_err(|_| SshError::KeyboardInteractiveUnavailable)?;
                let responses = tokio::time::timeout(timeout, answer)
                    .await
                    .map_err(|_| SshError::KeyboardInteractiveTimeout)?
                    .map_err(|_| SshError::KeyboardInteractiveUnavailable)?;
                if responses.len() != expected {
                    return Err(SshError::KeyboardInteractiveResponseCount {
                        expected,
                        actual: responses.len(),
                    });
                }
                response = tokio::time::timeout(
                    timeout,
                    handle.authenticate_keyboard_interactive_respond(responses),
                )
                .await
                .map_err(|_| SshError::KeyboardInteractiveTimeout)??;
            }
        }
    }
}

#[cfg(unix)]
async fn authenticate_agent(
    handle: &mut client::Handle<HostKeyVerifier>,
    user: &str,
) -> Result<client::AuthResult, SshError> {
    let mut agent = AgentClient::connect_env()
        .await
        .map_err(SshError::Agent)?
        .dynamic();
    if let Some(binding) = handle.agent_session_binding().cloned() {
        agent
            .bind_session(&binding, false)
            .await
            .map_err(SshError::Agent)?;
    }
    let identities = agent.request_identities().await.map_err(SshError::Agent)?;
    if identities.is_empty() {
        return Err(SshError::AuthenticationRejected);
    }

    for identity in identities {
        let hash = handle.best_supported_rsa_hash().await?.flatten();
        let result = match identity {
            AgentIdentity::PublicKey { key, .. } => {
                handle
                    .authenticate_publickey_with(user.to_owned(), key, hash, &mut agent)
                    .await
            }
            AgentIdentity::Certificate { certificate, .. } => {
                handle
                    .authenticate_certificate_with(user.to_owned(), certificate, hash, &mut agent)
                    .await
            }
        }
        .map_err(|error| {
            SshError::UnsupportedAuthentication(format!("SSH agent signing failed: {error}"))
        })?;
        if result.success() {
            return Ok(result);
        }
    }
    Err(SshError::AuthenticationRejected)
}

#[cfg(windows)]
async fn authenticate_agent(
    handle: &mut client::Handle<HostKeyVerifier>,
    user: &str,
) -> Result<client::AuthResult, SshError> {
    let mut agent = AgentClient::connect_pageant()
        .await
        .map_err(SshError::Agent)?
        .dynamic();
    if let Some(binding) = handle.agent_session_binding().cloned() {
        agent
            .bind_session(&binding, false)
            .await
            .map_err(SshError::Agent)?;
    }
    let identities = agent.request_identities().await.map_err(SshError::Agent)?;
    if identities.is_empty() {
        return Err(SshError::AuthenticationRejected);
    }

    for identity in identities {
        let hash = handle.best_supported_rsa_hash().await?.flatten();
        let result = match identity {
            AgentIdentity::PublicKey { key, .. } => {
                handle
                    .authenticate_publickey_with(user.to_owned(), key, hash, &mut agent)
                    .await
            }
            AgentIdentity::Certificate { certificate, .. } => {
                handle
                    .authenticate_certificate_with(user.to_owned(), certificate, hash, &mut agent)
                    .await
            }
        }
        .map_err(|error| {
            SshError::UnsupportedAuthentication(format!("SSH agent signing failed: {error}"))
        })?;
        if result.success() {
            return Ok(result);
        }
    }
    Err(SshError::AuthenticationRejected)
}

#[cfg(not(any(unix, windows)))]
async fn authenticate_agent(
    _handle: &mut client::Handle<HostKeyVerifier>,
    _user: &str,
) -> Result<client::AuthResult, SshError> {
    Err(SshError::UnsupportedAuthentication(AUTH_AGENT.to_owned()))
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
        assert_eq!(options.host_key_prompt_timeout, Duration::from_secs(120));
        assert!(options.host_key_prompt.is_none());
        assert!(options.keyboard_interactive_prompt.is_none());
        assert_eq!(
            options.keyboard_interactive_timeout,
            Duration::from_secs(120)
        );
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

    #[tokio::test]
    async fn socks5_proxy_opens_domain_tunnel() {
        use tokio::{
            io::{AsyncReadExt, AsyncWriteExt},
            net::TcpListener,
        };

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut greeting = [0u8; 3];
            stream.read_exact(&mut greeting).await.unwrap();
            assert_eq!(greeting, [5, 1, 0]);
            stream.write_all(&[5, 0]).await.unwrap();

            let mut request = [0u8; 5];
            stream.read_exact(&mut request).await.unwrap();
            assert_eq!(&request[..4], &[5, 1, 0, 3]);
            let mut host = vec![0u8; request[4] as usize];
            stream.read_exact(&mut host).await.unwrap();
            assert_eq!(host, b"ssh.example");
            let mut port = [0u8; 2];
            stream.read_exact(&mut port).await.unwrap();
            assert_eq!(u16::from_be_bytes(port), 2222);

            stream
                .write_all(&[5, 0, 0, 1, 127, 0, 0, 1, 0, 0])
                .await
                .unwrap();
            stream.write_all(b"SSH").await.unwrap();
        });

        let proxy = crate::config::ProxyConfig {
            kind: "socks5".into(),
            addr: "127.0.0.1".into(),
            port: address.port(),
            ..Default::default()
        };
        let mut stream = dial_socks5(&proxy, None, "ssh.example", 2222)
            .await
            .unwrap();
        let mut banner = [0u8; 3];
        stream.read_exact(&mut banner).await.unwrap();
        assert_eq!(&banner, b"SSH");
        server.await.unwrap();
    }

    #[tokio::test]
    async fn http_connect_proxy_sends_basic_auth_and_preserves_tunnel() {
        use tokio::{
            io::{AsyncReadExt, AsyncWriteExt},
            net::TcpListener,
        };

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let mut byte = [0u8; 1];
            while !request.ends_with(b"\r\n\r\n") {
                stream.read_exact(&mut byte).await.unwrap();
                request.push(byte[0]);
            }
            let request = String::from_utf8(request).unwrap();
            assert!(request.contains("CONNECT ssh.example:22 HTTP/1.1"));
            assert!(request.contains("Proxy-Authorization: Basic dXNlcjpwYXNz"));
            stream
                .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\nSSH")
                .await
                .unwrap();
        });

        let proxy = crate::config::ProxyConfig {
            kind: "http".into(),
            addr: "127.0.0.1".into(),
            port: address.port(),
            user: "user".into(),
            ..Default::default()
        };
        let mut stream = dial_http_connect(&proxy, Some("pass"), "ssh.example", 22)
            .await
            .unwrap();
        let mut banner = [0u8; 3];
        stream.read_exact(&mut banner).await.unwrap();
        assert_eq!(&banner, b"SSH");
        server.await.unwrap();
    }

    #[test]
    fn base64_encoder_handles_padding() {
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
    }
}
