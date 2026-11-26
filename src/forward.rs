use std::{net::SocketAddr, sync::{Arc, atomic::{AtomicU64, Ordering}}, time::Duration};

use tokio::{
    io::{AsyncReadExt, AsyncWriteExt, copy_bidirectional},
    net::{TcpListener, TcpStream},
    task::JoinHandle,
    sync::watch,
};

use crate::ssh::{HostKeyVerifier, SshError};

type SshHandle = russh::client::Handle<HostKeyVerifier>;

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
        Self { kind: ForwardKind::Local, listen, target_host: Some(target_host.into()), target_port: Some(target_port) }
    }

    pub fn dynamic(listen: SocketAddr) -> Self {
        Self { kind: ForwardKind::Dynamic, listen, target_host: None, target_port: None }
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
}

impl ForwardHandle {
    pub fn local_addr(&self) -> SocketAddr { self.addr }

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
        if let Some(task) = self.task.take() { let _ = task.await; }
    }
}

pub async fn start(handle: SshHandle, spec: ForwardSpec) -> Result<ForwardHandle, SshError> {
    if spec.kind == ForwardKind::Local && (spec.target_host.as_deref().unwrap_or("").is_empty() || spec.target_port.unwrap_or(0) == 0) {
        return Err(SshError::Proxy("local forwarding requires a target host and non-zero port".into()));
    }
    let listener = TcpListener::bind(spec.listen).await
        .map_err(|e| SshError::Proxy(format!("cannot listen on {}: {e}", spec.listen)))?;
    let addr = listener.local_addr().map_err(|e| SshError::Proxy(e.to_string()))?;
    let counters = Arc::new(Counters::default());
    let (shutdown, rx) = watch::channel(false);
    let task_counters = counters.clone();
    let task = tokio::spawn(async move {
        accept_loop(handle, listener, spec, task_counters, rx).await;
    });
    Ok(ForwardHandle { addr, counters, shutdown, task: Some(task) })
}

async fn accept_loop(handle: SshHandle, listener: TcpListener, spec: ForwardSpec, counters: Arc<Counters>, mut shutdown: watch::Receiver<bool>) {
    loop {
        tokio::select! {
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() { break; }
            }
            accepted = listener.accept() => match accepted {
                Ok((stream, peer)) => {
                    counters.total.fetch_add(1, Ordering::Relaxed);
                    counters.active.fetch_add(1, Ordering::Relaxed);
                    let h = handle.clone();
                    let c = counters.clone();
                    let target = spec.target_host.clone();
                    let port = spec.target_port;
                    tokio::spawn(async move {
                        let _active = ActiveGuard(c.clone());
                        let _ = tokio::time::timeout(Duration::from_secs(30), async {
                            match spec.kind {
                                ForwardKind::Local => {
                                    let Some(host) = target else { return; };
                                    let Some(port) = port else { return; };
                                    bridge(h, stream, host, port, peer, c).await;
                                }
                                ForwardKind::Dynamic => {
                                    dynamic_bridge(h, stream, peer, c).await;
                                }
                            }
                        }).await;
                    });
                }
                Err(_) => break,
            }
        }
    }
}

struct ActiveGuard(Arc<Counters>);
impl Drop for ActiveGuard {
    fn drop(&mut self) { self.0.active.fetch_sub(1, Ordering::Relaxed); }
}

async fn bridge(handle: SshHandle, mut local: TcpStream, host: String, port: u16, peer: SocketAddr, counters: Arc<Counters>) {
    let Ok(channel) = tokio::time::timeout(Duration::from_secs(15), handle.channel_open_direct_tcpip(host, port as u32, peer.ip().to_string(), peer.port() as u32)).await else { return; };
    let Ok(channel) = channel else { return; };
    let mut remote = channel.into_stream();
    let before_in = counters.uploaded.load(Ordering::Relaxed);
    let before_out = counters.downloaded.load(Ordering::Relaxed);
    if copy_bidirectional(&mut local, &mut remote).await.is_ok() {
        // Per-connection byte totals are accounted by the stream wrapper in a later stats refinement.
        let _ = (before_in, before_out);
    }
}

async fn dynamic_bridge(handle: SshHandle, mut local: TcpStream, peer: SocketAddr, counters: Arc<Counters>) {
    let result = async {
        let mut greeting = [0u8; 2];
        local.read_exact(&mut greeting).await?;
        if greeting[0] != 5 { return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "SOCKS version")); }
        let mut methods = vec![0; greeting[1] as usize];
        local.read_exact(&mut methods).await?;
        if !methods.contains(&0) {
            local.write_all(&[5, 0xff]).await?;
            return Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "SOCKS authentication required"));
        }
        local.write_all(&[5, 0]).await?;
        let mut request = [0u8; 4];
        local.read_exact(&mut request).await?;
        if request[0] != 5 || request[1] != 1 {
            local.write_all(&[5, 7, 0, 1, 0, 0, 0, 0, 0, 0]).await?;
            return Err(std::io::Error::new(std::io::ErrorKind::Unsupported, "SOCKS command"));
        }
        let host = match request[3] {
            1 => {
                let mut ip = [0u8; 4]; local.read_exact(&mut ip).await?;
                std::net::Ipv4Addr::from(ip).to_string()
            }
            4 => {
                let mut ip = [0u8; 16]; local.read_exact(&mut ip).await?;
                std::net::Ipv6Addr::from(ip).to_string()
            }
            3 => {
                let mut len = [0u8; 1]; local.read_exact(&mut len).await?;
                let mut name = vec![0; len[0] as usize]; local.read_exact(&mut name).await?;
                String::from_utf8(name).map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidData, "SOCKS hostname"))?
            }
            _ => {
                local.write_all(&[5, 8, 0, 1, 0, 0, 0, 0, 0, 0]).await?;
                return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "SOCKS address type"));
            }
        };
        let mut port = [0u8; 2]; local.read_exact(&mut port).await?;
        let port = u16::from_be_bytes(port);
        let opened = tokio::time::timeout(Duration::from_secs(15), handle.channel_open_direct_tcpip(host, port as u32, peer.ip().to_string(), peer.port() as u32)).await;
        let channel = match opened {
            Ok(Ok(channel)) => channel,
            _ => {
                local.write_all(&[5, 5, 0, 1, 0, 0, 0, 0, 0, 0]).await?;
                return Err(std::io::Error::new(std::io::ErrorKind::ConnectionRefused, "SSH destination refused"));
            }
        };
        local.write_all(&[5, 0, 0, 1, 0, 0, 0, 0, 0, 0]).await?;
        let mut remote = channel.into_stream();
        let _ = copy_bidirectional(&mut local, &mut remote).await?;
        Ok::<(), std::io::Error>(())
    }.await;
    if result.is_err() { counters.downloaded.fetch_add(0, Ordering::Relaxed); }
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
}
