use std::{
    fmt,
    path::{Path, PathBuf},
};

use russh_sftp::{client::SftpSession, protocol::OpenFlags};
use tokio::io::AsyncWriteExt;

use crate::{
    config::Host,
    ssh::{SshClient, SshCredentials, SshError, SshOptions},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteFileType {
    Directory,
    File,
    Symlink,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteEntry {
    pub name: String,
    pub path: String,
    pub file_type: RemoteFileType,
    pub size: u64,
    pub permissions: String,
    pub modified_at: Option<u32>,
}

#[derive(Debug)]
pub enum SftpError {
    Ssh(SshError),
    Protocol(String),
    Io(std::io::Error),
}

impl fmt::Display for SftpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ssh(error) => write!(f, "{error}"),
            Self::Protocol(error) => write!(f, "SFTP: {error}"),
            Self::Io(error) => write!(f, "local file I/O: {error}"),
        }
    }
}

impl std::error::Error for SftpError {}

impl From<SshError> for SftpError {
    fn from(error: SshError) -> Self {
        Self::Ssh(error)
    }
}

impl From<std::io::Error> for SftpError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

pub struct SftpClient {
    ssh: SshClient,
    session: SftpSession,
}

impl SftpClient {
    pub async fn connect(
        host: &Host,
        credentials: SshCredentials,
        options: SshOptions,
    ) -> Result<Self, SftpError> {
        let ssh = SshClient::connect(host, credentials, options).await?;
        let session = ssh.open_sftp().await?;
        session.set_timeout(30);
        Ok(Self { ssh, session })
    }

    pub async fn canonicalize(&self, path: &str) -> Result<String, SftpError> {
        self.session
            .canonicalize(path)
            .await
            .map_err(|error| SftpError::Protocol(error.to_string()))
    }

    pub async fn list_dir(&self, path: &str) -> Result<Vec<RemoteEntry>, SftpError> {
        let entries = self
            .session
            .read_dir(path)
            .await
            .map_err(|error| SftpError::Protocol(error.to_string()))?;
        Ok(entries
            .map(|entry| {
                let metadata = entry.metadata();
                let file_type = match entry.file_type() {
                    russh_sftp::protocol::FileType::Dir => RemoteFileType::Directory,
                    russh_sftp::protocol::FileType::File => RemoteFileType::File,
                    russh_sftp::protocol::FileType::Symlink => RemoteFileType::Symlink,
                    russh_sftp::protocol::FileType::Other => RemoteFileType::Other,
                };
                RemoteEntry {
                    name: entry.file_name().to_string(),
                    path: entry.path().to_string(),
                    file_type,
                    size: metadata.len(),
                    permissions: metadata.permissions().to_string(),
                    modified_at: metadata.mtime,
                }
            })
            .collect())
    }

    pub async fn metadata(&self, path: &str) -> Result<RemoteEntry, SftpError> {
        let metadata = self
            .session
            .metadata(path)
            .await
            .map_err(|error| SftpError::Protocol(error.to_string()))?;
        let file_type = match metadata.file_type() {
            russh_sftp::protocol::FileType::Dir => RemoteFileType::Directory,
            russh_sftp::protocol::FileType::File => RemoteFileType::File,
            russh_sftp::protocol::FileType::Symlink => RemoteFileType::Symlink,
            russh_sftp::protocol::FileType::Other => RemoteFileType::Other,
        };
        Ok(RemoteEntry {
            name: Path::new(path)
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.to_owned()),
            path: path.to_owned(),
            file_type,
            size: metadata.len(),
            permissions: metadata.permissions().to_string(),
            modified_at: metadata.mtime,
        })
    }

    pub async fn read_file(&self, path: &str) -> Result<Vec<u8>, SftpError> {
        self.session
            .read(path)
            .await
            .map_err(|error| SftpError::Protocol(error.to_string()))
    }

    pub async fn write_file(&self, path: &str, data: &[u8]) -> Result<(), SftpError> {
        self.session
            .write(path, data)
            .await
            .map_err(|error| SftpError::Protocol(error.to_string()))
    }

    pub async fn upload(
        &self,
        local_path: impl Into<PathBuf>,
        remote_path: &str,
    ) -> Result<u64, SftpError> {
        let mut local = tokio::fs::File::open(local_path.into()).await?;
        let mut remote = self
            .session
            .open_with_flags(
                remote_path,
                OpenFlags::CREATE | OpenFlags::TRUNCATE | OpenFlags::WRITE,
            )
            .await
            .map_err(|error| SftpError::Protocol(error.to_string()))?;
        let transferred = tokio::io::copy(&mut local, &mut remote).await?;
        remote.shutdown().await?;
        Ok(transferred)
    }

    pub async fn download(
        &self,
        remote_path: &str,
        local_path: impl Into<PathBuf>,
    ) -> Result<u64, SftpError> {
        let mut remote = self
            .session
            .open(remote_path)
            .await
            .map_err(|error| SftpError::Protocol(error.to_string()))?;
        let mut local = tokio::fs::File::create(local_path.into()).await?;
        let transferred = tokio::io::copy(&mut remote, &mut local).await?;
        remote.shutdown().await?;
        local.flush().await?;
        Ok(transferred)
    }

    pub async fn create_dir(&self, path: &str) -> Result<(), SftpError> {
        self.session
            .create_dir(path)
            .await
            .map_err(|error| SftpError::Protocol(error.to_string()))
    }

    pub async fn remove_file(&self, path: &str) -> Result<(), SftpError> {
        self.session
            .remove_file(path)
            .await
            .map_err(|error| SftpError::Protocol(error.to_string()))
    }

    pub async fn remove_dir(&self, path: &str) -> Result<(), SftpError> {
        self.session
            .remove_dir(path)
            .await
            .map_err(|error| SftpError::Protocol(error.to_string()))
    }

    pub async fn rename(&self, from: &str, to: &str) -> Result<(), SftpError> {
        self.session
            .rename(from, to)
            .await
            .map_err(|error| SftpError::Protocol(error.to_string()))
    }

    pub async fn read_link(&self, path: &str) -> Result<String, SftpError> {
        self.session
            .read_link(path)
            .await
            .map_err(|error| SftpError::Protocol(error.to_string()))
    }

    pub async fn symlink(&self, target: &str, link: &str) -> Result<(), SftpError> {
        self.session
            .symlink(target, link)
            .await
            .map_err(|error| SftpError::Protocol(error.to_string()))
    }

    pub async fn close(self) -> Result<(), SftpError> {
        self.session
            .close()
            .await
            .map_err(|error| SftpError::Protocol(error.to_string()))?;
        self.ssh.disconnect().await.map_err(SftpError::Ssh)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_file_type_is_explicit() {
        assert_ne!(RemoteFileType::Directory, RemoteFileType::File);
        assert_ne!(RemoteFileType::Symlink, RemoteFileType::Other);
    }
}
