use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use crate::{
    config::{
        AUTH_AGENT, AUTH_KEY, AUTH_KEYBOARD_INTERACTIVE, AUTH_PASSWORD, ConfigError, Host, Store,
    },
    importer::parse_ssh_config,
    ssh::{SshClient, SshCredentials, SshError, SshOptions},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionError {
    Config(String),
    Ssh(String),
    HostNotFound(String),
    HostDeleted(String),
    AlreadyConnected(String),
    HostStillConnected(String),
    MissingCredential(&'static str),
}

impl std::fmt::Display for ConnectionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Config(message) | Self::Ssh(message) => f.write_str(message),
            Self::HostNotFound(id) => write!(f, "SSH host profile not found: {id}"),
            Self::HostDeleted(id) => write!(f, "SSH host profile is deleted: {id}"),
            Self::AlreadyConnected(id) => write!(f, "SSH host is already connected: {id}"),
            Self::HostStillConnected(id) => {
                write!(
                    f,
                    "disconnect SSH host before permanently removing it: {id}"
                )
            }
            Self::MissingCredential(kind) => {
                write!(f, "credential is required for SSH authentication: {kind}")
            }
        }
    }
}

impl std::error::Error for ConnectionError {}

impl From<ConfigError> for ConnectionError {
    fn from(error: ConfigError) -> Self {
        Self::Config(error.to_string())
    }
}

impl From<SshError> for ConnectionError {
    fn from(error: SshError) -> Self {
        Self::Ssh(error.to_string())
    }
}

/// Saved SSH profiles and live connections. Secrets are supplied at connect time only.
pub struct ConnectionManager {
    config_path: PathBuf,
    store: Store,
    clients: BTreeMap<String, SshClient>,
}

impl ConnectionManager {
    pub fn open(path: impl Into<PathBuf>) -> Result<Self, ConfigError> {
        let config_path = path.into();
        let store = Store::load(&config_path)?;
        Ok(Self {
            config_path,
            store,
            clients: BTreeMap::new(),
        })
    }

    pub fn from_store(path: impl Into<PathBuf>, store: Store) -> Self {
        Self {
            config_path: path.into(),
            store,
            clients: BTreeMap::new(),
        }
    }

    pub fn store(&self) -> &Store {
        &self.store
    }

    pub fn config_path(&self) -> &Path {
        &self.config_path
    }

    pub fn connected_ids(&self) -> Vec<&str> {
        self.clients.keys().map(String::as_str).collect()
    }

    pub fn client(&self, host_id: &str) -> Option<&SshClient> {
        self.clients.get(host_id)
    }

    /// Import OpenSSH client profiles. Existing targets are left untouched;
    /// passwords are never read from or written to the SSH config.
    pub fn import_ssh_config(
        &mut self,
        text: &str,
        local_user: &str,
    ) -> Result<Vec<String>, ConnectionError> {
        let entries = parse_ssh_config(text, local_user)
            .map_err(|error| ConnectionError::Config(error.to_string()))?;
        let previous = self.store.clone();
        let mut imported = Vec::new();
        let mut aliases = BTreeMap::<String, String>::new();
        let mut pending = Vec::<(String, String)>::new();

        for entry in entries {
            let host = entry.host;
            let existing = self
                .store
                .hosts
                .iter()
                .find(|candidate| {
                    !candidate.deleted()
                        && candidate.addr.eq_ignore_ascii_case(&host.addr)
                        && candidate.port_or_22() == host.port_or_22()
                        && candidate.user == host.user
                })
                .map(|candidate| candidate.id.clone());
            let id = if let Some(id) = existing {
                id
            } else {
                self.store.add(host.clone());
                let id = self.store.hosts.last().expect("host just added").id.clone();
                imported.push(id.clone());
                id
            };
            aliases.insert(host.name.to_ascii_lowercase(), id.clone());
            if let Some(jump) = entry.proxy_jump {
                pending.push((id, jump));
            }
        }

        for (id, alias) in pending {
            let jump_id = aliases
                .get(&alias.to_ascii_lowercase())
                .cloned()
                .or_else(|| {
                    self.store
                        .hosts
                        .iter()
                        .find(|host| host.name.eq_ignore_ascii_case(&alias))
                        .map(|host| host.id.clone())
                });
            if let Some(jump_id) = jump_id.filter(|jump_id| jump_id != &id) {
                if let Some(host) = self.store.host_mut(&id) {
                    host.proxy = Some(crate::config::ProxyConfig {
                        kind: "jump".into(),
                        jump_host: jump_id,
                        ..Default::default()
                    });
                }
            }
        }

        if let Err(error) = self.save() {
            self.store = previous;
            return Err(error.into());
        }
        Ok(imported)
    }

    /// Import FinalShell's native connection directory without importing encrypted secrets.
    pub fn import_finalshell_dir(
        &mut self,
        path: impl AsRef<Path>,
    ) -> Result<Vec<String>, ConnectionError> {
        let entries = crate::importer::read_finalshell_dir(path)
            .map_err(|error| ConnectionError::Config(error.to_string()))?;
        let previous = self.store.clone();
        let mut imported = Vec::new();
        let mut folders = BTreeMap::<String, String>::new();
        for entry in entries {
            let host = entry.host;
            let existing = self
                .store
                .hosts
                .iter()
                .find(|candidate| {
                    !candidate.deleted()
                        && candidate.addr.eq_ignore_ascii_case(&host.addr)
                        && candidate.port_or_22() == host.port_or_22()
                        && candidate.user == host.user
                })
                .map(|candidate| candidate.id.clone());
            if existing.is_some() {
                continue;
            }
            let mut parent = String::new();
            for name in entry.folders {
                let key = format!("{parent}/{name}");
                parent = if let Some(id) = folders.get(&key) {
                    id.clone()
                } else {
                    let id = self.store.add_folder(name, parent);
                    folders.insert(key, id.clone());
                    id
                };
            }
            let mut host = host;
            host.folder = parent;
            self.store.add(host);
            imported.push(self.store.hosts.last().expect("host added").id.clone());
        }
        if let Err(error) = self.save() {
            self.store = previous;
            return Err(error.into());
        }
        Ok(imported)
    }

    /// Import Xshell .xsh sessions; existing targets are not duplicated.
    pub fn import_xshell(
        &mut self,
        path: impl AsRef<Path>,
    ) -> Result<Vec<String>, ConnectionError> {
        let entries = crate::importer::read_xshell(path)
            .map_err(|error| ConnectionError::Config(error.to_string()))?;
        let previous = self.store.clone();
        let mut imported = Vec::new();
        let mut folders = BTreeMap::<String, String>::new();
        for entry in entries {
            let mut parent = String::new();
            for name in entry.folders {
                let key = format!("{parent}/{name}");
                parent = if let Some(id) = folders.get(&key) {
                    id.clone()
                } else {
                    let id = self.store.add_folder(name, parent);
                    folders.insert(key, id.clone());
                    id
                };
            }
            let mut host = entry.host;
            let exists = self.store.hosts.iter().any(|candidate| {
                !candidate.deleted()
                    && candidate.addr.eq_ignore_ascii_case(&host.addr)
                    && candidate.port_or_22() == host.port_or_22()
                    && candidate.user == host.user
            });
            if exists {
                continue;
            }
            host.folder = parent;
            self.store.add(host);
            imported.push(self.store.hosts.last().expect("host added").id.clone());
        }
        if let Err(error) = self.save() {
            self.store = previous;
            return Err(error.into());
        }
        Ok(imported)
    }

    pub fn import_mobaxterm(
        &mut self,
        path: impl AsRef<Path>,
    ) -> Result<Vec<String>, ConnectionError> {
        let entries = crate::importer::read_mobaxterm(path)
            .map_err(|error| ConnectionError::Config(error.to_string()))?;
        let previous = self.store.clone();
        let mut imported = Vec::new();
        let mut folders = BTreeMap::<String, String>::new();
        for entry in entries {
            let mut parent = String::new();
            for name in entry.folders {
                let key = format!("{parent}/{name}");
                parent = if let Some(id) = folders.get(&key) {
                    id.clone()
                } else {
                    let id = self.store.add_folder(name, parent);
                    folders.insert(key, id.clone());
                    id
                };
            }
            let mut host = entry.host;
            if self.store.hosts.iter().any(|candidate| {
                !candidate.deleted()
                    && candidate.addr.eq_ignore_ascii_case(&host.addr)
                    && candidate.port_or_22() == host.port_or_22()
                    && candidate.user == host.user
            }) {
                continue;
            }
            host.folder = parent;
            self.store.add(host);
            imported.push(self.store.hosts.last().expect("host added").id.clone());
        }
        if let Err(error) = self.save() {
            self.store = previous;
            return Err(error.into());
        }
        Ok(imported)
    }

    /// Import SecureCRT sessions from its Config or Sessions directory.
    pub fn import_securecrt(
        &mut self,
        path: impl AsRef<Path>,
    ) -> Result<Vec<String>, ConnectionError> {
        let entries = crate::importer::read_securecrt(path)
            .map_err(|error| ConnectionError::Config(error.to_string()))?;
        let previous = self.store.clone();
        let mut imported = Vec::new();
        let mut folders = BTreeMap::<String, String>::new();
        for entry in entries {
            let mut parent = String::new();
            for name in entry.folders {
                let key = format!("{parent}/{name}");
                parent = if let Some(id) = folders.get(&key) {
                    id.clone()
                } else {
                    let id = self.store.add_folder(name, parent);
                    folders.insert(key, id.clone());
                    id
                };
            }
            let mut host = entry.host;
            if self.store.hosts.iter().any(|candidate| {
                !candidate.deleted()
                    && candidate.addr.eq_ignore_ascii_case(&host.addr)
                    && candidate.port_or_22() == host.port_or_22()
                    && candidate.user == host.user
            }) {
                continue;
            }
            host.folder = parent;
            self.store.add(host);
            imported.push(self.store.hosts.last().expect("host added").id.clone());
        }
        if let Err(error) = self.save() {
            self.store = previous;
            return Err(error.into());
        }
        Ok(imported)
    }

    pub fn save(&self) -> Result<(), ConfigError> {
        self.store.save(&self.config_path)
    }

    pub fn add_host(&mut self, host: Host) -> Result<String, ConnectionError> {
        self.store.add(host);
        let id = self
            .store
            .hosts
            .last()
            .expect("host was just added")
            .id
            .clone();
        if let Err(error) = self.save() {
            self.store.remove(&id);
            return Err(error.into());
        }
        Ok(id)
    }

    pub fn update_host(&mut self, host_id: &str, mut updated: Host) -> Result<(), ConnectionError> {
        let Some(existing) = self.store.host(host_id) else {
            return Err(ConnectionError::HostNotFound(host_id.into()));
        };
        if existing.deleted() {
            return Err(ConnectionError::HostDeleted(host_id.into()));
        }
        updated.id = host_id.into();
        let previous = existing.clone();
        *self.store.host_mut(host_id).expect("host exists") = updated;
        if let Err(error) = self.save() {
            *self.store.host_mut(host_id).expect("host exists") = previous;
            return Err(error.into());
        }
        Ok(())
    }

    pub fn add_folder(
        &mut self,
        name: impl Into<String>,
        parent: impl Into<String>,
    ) -> Result<String, ConnectionError> {
        let id = self.store.add_folder(name, parent);
        if let Err(error) = self.save() {
            self.store.remove_folder(&id);
            return Err(error.into());
        }
        Ok(id)
    }

    pub fn remove_folder(&mut self, folder_id: &str) -> Result<bool, ConnectionError> {
        let previous = self.store.clone();
        if !self.store.remove_folder(folder_id) {
            return Ok(false);
        }
        if let Err(error) = self.save() {
            self.store = previous;
            return Err(error.into());
        }
        Ok(true)
    }

    pub fn soft_delete_host(&mut self, host_id: &str) -> Result<bool, ConnectionError> {
        let previous = self.store.clone();
        if !self.store.soft_delete(host_id, utc_timestamp()) {
            return Ok(false);
        }
        if let Err(error) = self.save() {
            self.store = previous;
            return Err(error.into());
        }
        Ok(true)
    }

    pub fn restore_host(&mut self, host_id: &str) -> Result<bool, ConnectionError> {
        let previous = self.store.clone();
        if !self.store.restore(host_id) {
            return Ok(false);
        }
        if let Err(error) = self.save() {
            self.store = previous;
            return Err(error.into());
        }
        Ok(true)
    }

    pub fn remove_host(&mut self, host_id: &str) -> Result<bool, ConnectionError> {
        if self.clients.contains_key(host_id) {
            return Err(ConnectionError::HostStillConnected(host_id.into()));
        }
        let previous = self.store.clone();
        if !self.store.remove(host_id) {
            return Ok(false);
        }
        if let Err(error) = self.save() {
            self.store = previous;
            return Err(error.into());
        }
        Ok(true)
    }

    pub async fn connect_host(
        &mut self,
        host_id: &str,
        secret: Option<String>,
        options: SshOptions,
    ) -> Result<(), ConnectionError> {
        if self.clients.contains_key(host_id) {
            return Err(ConnectionError::AlreadyConnected(host_id.into()));
        }
        let host = self
            .store
            .host(host_id)
            .cloned()
            .ok_or_else(|| ConnectionError::HostNotFound(host_id.into()))?;
        if host.deleted() {
            return Err(ConnectionError::HostDeleted(host_id.into()));
        }

        let credentials = match host.auth.as_str() {
            AUTH_PASSWORD => SshCredentials::Password(
                secret.ok_or(ConnectionError::MissingCredential("password"))?,
            ),
            AUTH_KEY => {
                if host.key_path.is_empty() {
                    return Err(ConnectionError::MissingCredential("private key path"));
                }
                SshCredentials::PrivateKey {
                    path: PathBuf::from(&host.key_path),
                    passphrase: secret,
                }
            }
            AUTH_AGENT | AUTH_KEYBOARD_INTERACTIVE => SshCredentials::KeyboardInteractive,
            kind => {
                return Err(ConnectionError::Ssh(
                    SshError::UnsupportedAuthentication(kind.into()).to_string(),
                ));
            }
        };

        let client = SshClient::connect(&host, credentials, options).await?;
        let previous_store = self.store.clone();
        self.clients.insert(host_id.into(), client);
        self.store.mark_connected(host_id, utc_timestamp());
        if let Err(error) = self.save() {
            self.store = previous_store;
            if let Some(client) = self.clients.remove(host_id) {
                let _ = client.disconnect().await;
            }
            return Err(error.into());
        }
        Ok(())
    }

    pub async fn disconnect_host(&mut self, host_id: &str) -> Result<bool, ConnectionError> {
        let Some(client) = self.clients.remove(host_id) else {
            return Ok(false);
        };
        client.disconnect().await?;
        Ok(true)
    }

    pub async fn disconnect_all(&mut self) {
        for (_, client) in std::mem::take(&mut self.clients) {
            let _ = client.disconnect().await;
        }
    }
}

fn utc_timestamp() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let days = (seconds / 86_400) as i64;
    let day_seconds = seconds % 86_400;
    let z = days + 719_468;
    let era = (if z >= 0 { z } else { z - 146_096 }) / 146_097;
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    let hour = day_seconds / 3_600;
    let minute = (day_seconds % 3_600) / 60;
    let second = day_seconds % 60;
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AUTH_PASSWORD;

    #[test]
    fn utc_timestamp_is_rfc3339() {
        let value = utc_timestamp();
        assert_eq!(value.len(), 20);
        assert!(value.ends_with('Z'));
        assert_eq!(&value[4..5], "-");
        assert_eq!(&value[7..8], "-");
    }

    #[test]
    fn profile_crud_persists_and_preserves_rollback_contract() {
        let dir = std::env::temp_dir().join(format!(
            "finalshell-session-{}",
            utc_timestamp().replace([':', '-'], "")
        ));
        let path = dir.join("hosts.json");
        let mut manager = ConnectionManager::from_store(&path, Store::default());
        let id = manager
            .add_host(Host {
                name: "Production".into(),
                addr: "example.com".into(),
                user: "root".into(),
                auth: AUTH_PASSWORD.into(),
                ..Host::default()
            })
            .unwrap();
        assert!(manager.store().host(&id).is_some());
        let saved = std::fs::read_to_string(&path).unwrap();
        assert!(saved.contains("example.com"));
        assert!(!saved.contains("password-secret"));
        manager
            .update_host(
                &id,
                Host {
                    name: "Updated".into(),
                    ..Host::default()
                },
            )
            .unwrap();
        assert_eq!(manager.store().host(&id).unwrap().name, "Updated");
        assert!(manager.soft_delete_host(&id).unwrap());
        assert!(manager.store().host(&id).unwrap().deleted());
        assert!(manager.restore_host(&id).unwrap());
        assert!(manager.remove_host(&id).unwrap());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
