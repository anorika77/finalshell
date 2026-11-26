use std::{
    net::SocketAddr,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use russh::{Channel, client};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt, copy_bidirectional},
    net::{TcpListener, TcpStream},
    sync::watch,
    task::JoinHandle,
};

use crate::ssh::{HostKeyVerifier, SshError};

type SshHandle = Arc<client::Handle<HostKeyVerifier>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForwardKind {
    Local,
    Dynamic,
}

#[derive(Debug, Clone)]
pub struct ForwardSpec {
    pub kind: ForwardKind,
    pub listen: SocketAddr,
    pub target_host: Option<String>,
    pub target_port: Option<u16>,
}

impl ForwardSpec {
    pub fn local(listen: SocketAddr, target_host: impl Into<String>, target_port: u16) -> Self {
        Self {
            kind: ForwardKind::Local,
            listen,
            target_host: Some(target_host.into()),
            target_port: Some(target_port),
        }
    }

    pub fn dynamic(listen: SocketAddr) -> Self {
        Self {
            kind: ForwardKind::Dynamic,
            listen,
            target_host: None,
            target_port: None,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ForwardStats {
    pub active: u64,
    pub total: u64,
    pub uploaded: u64,
    pub downloaded: u64,
}

#[derive(Default)]
struct Counters {
    active: AtomicU64,
    total: AtomicU64,
    uploaded: AtomicU64,
    downloaded: AtomicU64,
}

pub struct ForwardHandle {
    addr: SocketAddr,
    counters: Arc<Counters>,
    shutdown: watch::Sender<bool>,
    task: Option<JoinHandle<()>>,
    remote_cancel: Option<RemoteCancel>,
}

impl ForwardHandle {
    pub fn local_addr(&self) -> SocketAddr {
        self.addr
    }

    pub fn stats(&self) -> ForwardStats {
        ForwardStats {
            active: self.counters.active.load(Ordering::Relaxed),
            total: self.counters.total.load(Ordering::Relaxed),
            uploaded: self.counters.uploaded.load(Ordering::Relaxed),
            downloaded: self.counters.downloaded.load(Ordering::Relaxed),
        }
    }

    pub async fn close(mut self) {
        let _ = self.shutdown.send(true);
        if let Some(task) = self.task.take() {
            let _ = task.await;
        }
        if let Some(remote) = self.remote_cancel.take() {
            remote.routes.lock().await.remove(&remote.route_key);
            let _ = remote
                .handle
                .cancel_tcpip_forward(remote.address, remote.port as u32)
                .await;
        }
    }
}

pub(crate) async fn start(handle: SshHandle, spec: ForwardSpec) -> Result<ForwardHandle, SshError> {
    if spec.kind == ForwardKind::Local
        && (spec.target_host.as_deref().unwrap_or("").is_empty()
            || spec.target_port.unwrap_or(0) == 0)
    {
        return Err(SshError::Proxy(
            "local forwarding requires a target host and non-zero port".into(),
        ));
    }
    let listener = TcpListener::bind(spec.listen)
        .await
        .map_err(|e| SshError::Proxy(format!("cannot listen on {}: {e}", spec.listen)))?;
    let addr = listener
        .local_addr()
        .map_err(|e| SshError::Proxy(e.to_string()))?;
    let counters = Arc::new(Counters::default());
    let (shutdown, rx) = watch::channel(false);
    let task_counters = counters.clone();
    let task = tokio::spawn(async move {
        accept_loop(handle, listener, spec, task_counters, rx).await;
    });
    Ok(ForwardHandle {
        addr,
        counters,
        shutdown,
        task: Some(task),
        remote_cancel: None,
    })
}

async fn accept_loop(
    handle: SshHandle,
    listener: TcpListener,
    spec: ForwardSpec,
    counters: Arc<Counters>,
    mut shutdown: watch::Receiver<bool>,
) {
    loop {
        tokio::select! {
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    break;
                }
            }
            accepted = listener.accept() => match accepted {
                Ok((stream, peer)) => {
                    counters.total.fetch_add(1, Ordering::Relaxed);
                    counters.active.fetch_add(1, Ordering::Relaxed);
                    let h = handle.clone();
                    let c = counters.clone();
                    let target = spec.target_host.clone();
                    let port = spec.target_port;
                    let kind = spec.kind;
                    tokio::spawn(async move {
                        let _active = ActiveGuard(c.clone());
                        match kind {
                            ForwardKind::Local => {
                                if let (Some(host), Some(port)) = (target, port) {
                                    bridge(h, stream, host, port, peer, c).await;
                                }
                            }
                            ForwardKind::Dynamic => dynamic_bridge(h, stream, peer, c).await,
                        }
                    });
                }
                Err(_) => break,
            }
        }
    }
}

struct ActiveGuard(Arc<Counters>);

impl Drop for ActiveGuard {
    fn drop(&mut self) {
        self.0.active.fetch_sub(1, Ordering::Relaxed);
    }
}

async fn bridge(
    handle: SshHandle,
    mut local: TcpStream,
    host: String,
    port: u16,
    peer: SocketAddr,
    counters: Arc<Counters>,
) {
    let Ok(Ok(channel)) = tokio::time::timeout(
        Duration::from_secs(15),
        handle.channel_open_direct_tcpip(
            host,
            port as u32,
            peer.ip().to_string(),
            peer.port() as u32,
        ),
    )
    .await
    else {
        return;
    };
    let mut remote = channel.into_stream();
    if let Ok((up, down)) = copy_bidirectional(&mut local, &mut remote).await {
        counters.uploaded.fetch_add(up, Ordering::Relaxed);
        counters.downloaded.fetch_add(down, Ordering::Relaxed);
    }
}

async fn dynamic_bridge(
    handle: SshHandle,
    mut local: TcpStream,
    peer: SocketAddr,
    counters: Arc<Counters>,
) {
    let Ok(Ok(channel)) = tokio::time::timeout(
        Duration::from_secs(30),
        socks_open(&handle, &mut local, peer),
    )
    .await
    else {
        return;
    };
    let mut remote = channel.into_stream();
    if let Ok((up, down)) = copy_bidirectional(&mut local, &mut remote).await {
        counters.uploaded.fetch_add(up, Ordering::Relaxed);
        counters.downloaded.fetch_add(down, Ordering::Relaxed);
    }
}

async fn socks_open(
    handle: &SshHandle,
    local: &mut TcpStream,
    peer: SocketAddr,
) -> std::io::Result<Channel<client::Msg>> {
    let mut greeting = [0u8; 2];
    local.read_exact(&mut greeting).await?;
    if greeting[0] != 5 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "SOCKS version",
        ));
    }
    let mut methods = vec![0; greeting[1] as usize];
    local.read_exact(&mut methods).await?;
    if !methods.contains(&0) {
        local.write_all(&[5, 0xff]).await?;
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "SOCKS authentication required",
        ));
    }
    local.write_all(&[5, 0]).await?;
    let mut request = [0u8; 4];
    local.read_exact(&mut request).await?;
    if request[0] != 5 || request[1] != 1 {
        local.write_all(&[5, 7, 0, 1, 0, 0, 0, 0, 0, 0]).await?;
        return Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "SOCKS command",
        ));
    }
    let host = match request[3] {
        1 => {
            let mut ip = [0u8; 4];
            local.read_exact(&mut ip).await?;
            std::net::Ipv4Addr::from(ip).to_string()
        }
        4 => {
            let mut ip = [0u8; 16];
            local.read_exact(&mut ip).await?;
            std::net::Ipv6Addr::from(ip).to_string()
        }
        3 => {
            let mut len = [0u8; 1];
            local.read_exact(&mut len).await?;
            let mut name = vec![0; len[0] as usize];
            local.read_exact(&mut name).await?;
            String::from_utf8(name).map_err(|_| {
                std::io::Error::new(std::io::ErrorKind::InvalidData, "SOCKS hostname")
            })?
        }
        _ => {
            local.write_all(&[5, 8, 0, 1, 0, 0, 0, 0, 0, 0]).await?;
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "SOCKS address type",
            ));
        }
    };
    let mut port = [0u8; 2];
    local.read_exact(&mut port).await?;
    let port = u16::from_be_bytes(port);
    let channel = match handle
        .channel_open_direct_tcpip(host, port as u32, peer.ip().to_string(), peer.port() as u32)
        .await
    {
        Ok(channel) => channel,
        Err(_) => {
            local.write_all(&[5, 5, 0, 1, 0, 0, 0, 0, 0, 0]).await?;
            return Err(std::io::Error::new(
                std::io::ErrorKind::ConnectionRefused,
                "SSH destination refused",
            ));
        }
    };
    local.write_all(&[5, 0, 0, 1, 0, 0, 0, 0, 0, 0]).await?;
    Ok(channel)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forward_specs_keep_local_and_dynamic_semantics_distinct() {
        let listen = "127.0.0.1:0".parse().unwrap();
        let local = ForwardSpec::local(listen, "db.internal", 5432);
        let dynamic = ForwardSpec::dynamic(listen);
        assert_eq!(local.kind, ForwardKind::Local);
        assert_eq!(local.target_host.as_deref(), Some("db.internal"));
        assert_eq!(local.target_port, Some(5432));
        assert_eq!(dynamic.kind, ForwardKind::Dynamic);
        assert!(dynamic.target_host.is_none());
        assert!(dynamic.target_port.is_none());
    }

    #[test]
    fn remote_forward_listen_host_must_be_an_ip_or_localhost() {
        assert_eq!(
            remote_listen_ip("localhost").unwrap(),
            "127.0.0.1".parse().unwrap()
        );
        assert_eq!(remote_listen_ip("::1").unwrap(), "::1".parse().unwrap());
        assert!(remote_listen_ip("remote.example").is_err());
    }
}

#[derive(Debug, Clone)]
pub struct RemoteForwardSpec {
    pub listen_host: String,
    pub listen_port: u16,
    pub target_host: String,
    pub target_port: u16,
}

struct RemoteCancel {
    handle: SshHandle,
    routes: crate::ssh::RemoteForwardRoutes,
    address: String,
    port: u16,
    route_key: u16,
}

fn remote_listen_ip(host: &str) -> Result<std::net::IpAddr, SshError> {
    let host = if host.is_empty() || host.eq_ignore_ascii_case("localhost") {
        "127.0.0.1"
    } else {
        host
    };
    host.parse::<std::net::IpAddr>().map_err(|_| {
        SshError::Proxy("remote forwarding listen host must be an IP address or localhost".into())
    })
}

pub(crate) async fn start_remote(
    handle: SshHandle,
    routes: crate::ssh::RemoteForwardRoutes,
    spec: RemoteForwardSpec,
) -> Result<ForwardHandle, SshError> {
    if spec.target_host.is_empty() || spec.target_port == 0 {
        return Err(SshError::Proxy(
            "remote forwarding requires a target host and non-zero port".into(),
        ));
    }
    let listen_ip = remote_listen_ip(&spec.listen_host)?;
    let address = listen_ip.to_string();
    let route_key = spec.listen_port;
    let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
    {
        let mut registered = routes.lock().await;
        if registered.contains_key(&route_key) {
            return Err(SshError::Proxy(format!(
                "a remote forwarding is already registered for port {}",
                route_key
            )));
        }
        registered.insert(route_key, sender.clone());
    }

    let requested_port = spec.listen_port;
    let actual_port = match handle
        .tcpip_forward(address.clone(), requested_port as u32)
        .await
    {
        Ok(_) if requested_port > 0 => requested_port,
        Ok(port) if port > 0 && port <= u16::MAX as u32 => port as u16,
        Ok(_) => {
            routes.lock().await.remove(&route_key);
            return Err(SshError::Proxy(
                "SSH server returned an invalid remote port".into(),
            ));
        }
        Err(error) => {
            routes.lock().await.remove(&route_key);
            return Err(SshError::Transport(error));
        }
    };

    if route_key == 0 {
        let mut registered = routes.lock().await;
        registered.remove(&0);
        if registered.contains_key(&actual_port) {
            drop(registered);
            let _ = handle
                .cancel_tcpip_forward(address, actual_port as u32)
                .await;
            return Err(SshError::Proxy(format!(
                "remote forwarding port {actual_port} is already registered"
            )));
        }
        registered.insert(actual_port, sender);
    }

    let counters = Arc::new(Counters::default());
    let (shutdown, rx) = watch::channel(false);
    let task_counters = counters.clone();
    let target_host = spec.target_host;
    let target_port = spec.target_port;
    let task = tokio::spawn(async move {
        remote_accept_loop(receiver, target_host, target_port, task_counters, rx).await;
    });
    Ok(ForwardHandle {
        addr: SocketAddr::new(listen_ip, actual_port),
        counters,
        shutdown,
        task: Some(task),
        remote_cancel: Some(RemoteCancel {
            handle,
            routes,
            address,
            port: actual_port,
            route_key: actual_port,
        }),
    })
}

async fn remote_accept_loop(
    mut receiver: tokio::sync::mpsc::UnboundedReceiver<Channel<client::Msg>>,
    target_host: String,
    target_port: u16,
    counters: Arc<Counters>,
    mut shutdown: watch::Receiver<bool>,
) {
    loop {
        tokio::select! {
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    break;
                }
            }
            incoming = receiver.recv() => match incoming {
                Some(channel) => {
                    counters.total.fetch_add(1, Ordering::Relaxed);
                    counters.active.fetch_add(1, Ordering::Relaxed);
                    let host = target_host.clone();
                    let counters = counters.clone();
                    tokio::spawn(async move {
                        let _active = ActiveGuard(counters.clone());
                        let Ok(mut local) = TcpStream::connect((host.as_str(), target_port)).await else {
                            return;
                        };
                        let mut remote = channel.into_stream();
                        if let Ok((up, down)) = copy_bidirectional(&mut remote, &mut local).await {
                            counters.uploaded.fetch_add(up, Ordering::Relaxed);
                            counters.downloaded.fetch_add(down, Ordering::Relaxed);
                        }
                    });
                }
                None => break,
            }
        }
    }
}
