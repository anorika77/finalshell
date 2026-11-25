use std::{
    fs::{self, File, OpenOptions},

    io::Write,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use serde_json::{Map, Value};

pub const HOSTS_VERSION: u64 = 2;
pub const V1_BACKUP_NAME: &str = "hosts.v1.json";
static TEMP_ID: AtomicU64 = AtomicU64::new(0);

#[derive(Debug)]
pub enum ConfigError {
    Io(std::io::Error),
    Json(serde_json::Error),
    InvalidFormat(String),
}
impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self { Self::Io(e) => write!(f, "{e}"), Self::Json(e) => write!(f, "{e}") }
    }
}
impl std::error::Error for ConfigError {}
impl From<std::io::Error> for ConfigError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}
impl From<serde_json::Error> for ConfigError {
    fn from(e: serde_json::Error) -> Self {
        Self::Json(e)
    }
}

fn object(value: Value) -> Map<String, Value> {
    match value { Value::Object(m) => m, _ => Map::new() }
}
fn take_string(m: &mut Map<String, Value>, key: &str) -> String {
    m.remove(key).and_then(|v| v.as_str().map(str::to_owned)).unwrap_or_default()
}
fn take_u64(m: &mut Map<String, Value>, key: &str) -> u64 {
    m.remove(key).and_then(|v| v.as_u64()).unwrap_or_default()
}
fn take_bool(m: &mut Map<String, Value>, key: &str) -> bool {
    m.remove(key).and_then(|v| v.as_bool()).unwrap_or_default()
}
fn take_strings(m: &mut Map<String, Value>, key: &str) -> Vec<String> {
    m.remove(key)
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default()
        .into_iter()
        .filter_map(|v| v.as_str().map(str::to_owned))
        .collect()
}
fn put_string(m: &mut Map<String, Value>, key: &str, value: &str) {
    if !value.is_empty() { m.insert(key.into(), Value::String(value.into())); }
}
fn put_bool(m: &mut Map<String, Value>, key: &str, value: bool) {
    if value { m.insert(key.into(), Value::Bool(true)); }
}
fn put_number(m: &mut Map<String, Value>, key: &str, value: u64) {
    if value != 0 { m.insert(key.into(), Value::from(value)); }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ProxyConfig {
    pub kind: String,
    pub addr: String,
    pub port: u16,
    pub user: String,
    pub jump_host: String,
    pub extra: Map<String, Value>,
}
impl ProxyConfig {
    fn from_value(v: Value) -> Self {
        let mut m = object(v);
        Self {
            kind: take_string(&mut m, "kind"),
            addr: take_string(&mut m, "addr"),
            port: take_u64(&mut m, "port") as u16,
            user: take_string(&mut m, "user"),
            jump_host: take_string(&mut m, "jumpHost"),
            extra: m,
        }
    }
    fn to_value(&self) -> Value {
        let mut m = self.extra.clone();
        put_string(&mut m, "kind", &self.kind);
        put_string(&mut m, "addr", &self.addr);
        put_number(&mut m, "port", self.port as u64);
        put_string(&mut m, "user", &self.user);
        put_string(&mut m, "jumpHost", &self.jump_host);
        Value::Object(m)
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Tunnel {
    pub kind: String,
    pub listen_host: String,
    pub listen_port: u16,
    pub target_host: String,
    pub target_port: u16,
    pub disabled: bool,
    pub extra: Map<String, Value>,
}
impl Tunnel {
    fn from_value(v: Value) -> Self {
        let mut m = object(v);
        Self {
            kind: take_string(&mut m, "kind"),
            listen_host: take_string(&mut m, "listenHost"),
            listen_port: take_u64(&mut m, "listenPort") as u16,
            target_host: take_string(&mut m, "targetHost"),
            target_port: take_u64(&mut m, "targetPort") as u16,
            disabled: take_bool(&mut m, "disabled"),
            extra: m,
        }
    }
    fn to_value(&self) -> Value {
        let mut m = self.extra.clone();
        put_string(&mut m, "kind", &self.kind);
        put_string(&mut m, "listenHost", &self.listen_host);
        put_number(&mut m, "listenPort", self.listen_port as u64);
        put_string(&mut m, "targetHost", &self.target_host);
        put_number(&mut m, "targetPort", self.target_port as u64);
        put_bool(&mut m, "disabled", self.disabled);
        Value::Object(m)
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Host {
    pub id: String,
    pub name: String,
    pub addr: String,
    pub port: u16,
    pub user: String,
    pub auth: String,
    pub key_path: String,
    pub folder: String,
    pub notes: String,
    pub no_monitor: bool,
    pub last_connected_at: String,
    pub deleted_at: String,
    pub term_type: String,
    pub backspace: String,
    pub no_remember_password: bool,
    pub proxy: Option<ProxyConfig>,
    pub tunnels: Vec<Tunnel>,
    pub delete: String,
    pub encoding: String,
    pub created_at: String,
    pub monitor_ifaces: Vec<String>,
    pub extra: Map<String, Value>,
}
impl Host {
    pub fn from_value(v: Value) -> Self {
        let mut m = object(v);
        let proxy = m
            .remove("proxy")
            .filter(|v| !v.is_null())
            .map(ProxyConfig::from_value);
        let tunnels = m
            .remove("tunnels")
            .and_then(|v| v.as_array().cloned())
            .unwrap_or_default()
            .into_iter()
            .map(Tunnel::from_value)
            .collect();
        let monitor_ifaces = take_strings(&mut m, "monitorIfaces");
        Self {
            id: take_string(&mut m, "id"),
            name: take_string(&mut m, "name"),
            addr: take_string(&mut m, "addr"),
            port: take_u64(&mut m, "port") as u16,
            user: take_string(&mut m, "user"),
            auth: take_string(&mut m, "auth"),
            key_path: take_string(&mut m, "keyPath"),
            folder: take_string(&mut m, "folder"),
            notes: take_string(&mut m, "notes"),
            no_monitor: take_bool(&mut m, "noMonitor"),
            last_connected_at: take_string(&mut m, "lastConnectedAt"),
            deleted_at: take_string(&mut m, "deletedAt"),
            term_type: take_string(&mut m, "termType"),
            backspace: take_string(&mut m, "backspace"),
            no_remember_password: take_bool(&mut m, "noRememberPassword"),
            proxy,
            tunnels,
            delete: take_string(&mut m, "delete"),
            encoding: take_string(&mut m, "encoding"),
            created_at: take_string(&mut m, "createdAt"),
            monitor_ifaces,
            extra: m,
        }
    }
    pub fn to_value(&self) -> Value {
        let mut m = self.extra.clone();
        put_string(&mut m, "id", &self.id);
        put_string(&mut m, "name", &self.name);
        put_string(&mut m, "addr", &self.addr);
        put_number(&mut m, "port", self.port as u64);
        put_string(&mut m, "user", &self.user);
        put_string(&mut m, "auth", &self.auth);
        put_string(&mut m, "keyPath", &self.key_path);
        put_string(&mut m, "folder", &self.folder);
        put_string(&mut m, "notes", &self.notes);
        put_bool(&mut m, "noMonitor", self.no_monitor);
        put_string(&mut m, "lastConnectedAt", &self.last_connected_at);
        put_string(&mut m, "deletedAt", &self.deleted_at);
        put_string(&mut m, "termType", &self.term_type);
        put_string(&mut m, "backspace", &self.backspace);
        put_bool(&mut m, "noRememberPassword", self.no_remember_password);
        if let Some(proxy) = &self.proxy {
            m.insert("proxy".into(), proxy.to_value());
        }
        if !self.tunnels.is_empty() {
            m.insert(
                "tunnels".into(),
                Value::Array(self.tunnels.iter().map(Tunnel::to_value).collect()),
            );
        }
        put_string(&mut m, "delete", &self.delete);
        put_string(&mut m, "encoding", &self.encoding);
        put_string(&mut m, "createdAt", &self.created_at);
        if !self.monitor_ifaces.is_empty() {
            m.insert(
                "monitorIfaces".into(),
                serde_json::json!(self.monitor_ifaces),
            );
        }
        Value::Object(m)
    }
    pub fn title(&self) -> &str {
        if self.name.is_empty() {
            &self.addr
        } else {
            &self.name
        }
    }
    pub fn port_or_22(&self) -> u16 {
        if self.port == 0 {
            22
        } else {
            self.port
        }
    }
    pub fn target(&self) -> String {
        if self.port_or_22() == 22 { format!("{}@{}", self.user, self.addr) }
        else { format!("{}@{}:{}", self.user, self.addr, self.port_or_22()) }
    }
    pub fn deleted(&self) -> bool {
        !self.deleted_at.is_empty()
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Folder {
    pub id: String,
    pub name: String,
    pub parent: String,
    pub extra: Map<String, Value>,
}
impl Folder {
    fn from_value(v: Value) -> Self {
        let mut m = object(v);
        Self {
            id: take_string(&mut m, "id"),
            name: take_string(&mut m, "name"),
            parent: take_string(&mut m, "parent"),
            extra: m,
        }
    }
    fn to_value(&self) -> Value {
        let mut m = self.extra.clone();
        put_string(&mut m, "id", &self.id);
        put_string(&mut m, "name", &self.name);
        put_string(&mut m, "parent", &self.parent);
        Value::Object(m)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Store {
    pub version: u64,
    pub folders: Vec<Folder>,
    pub hosts: Vec<Host>,
    pub extra: Map<String, Value>,
}
impl Default for Store {
    fn default() -> Self {
        Self {
            version: HOSTS_VERSION,
            folders: Vec::new(),
            hosts: Vec::new(),
            extra: Map::new(),
        }
    }
}
impl Store {
    pub fn parse(data: &[u8]) -> Result<(Self, bool), ConfigError> {
        let value: Value = if data.iter().all(u8::is_ascii_whitespace) {
            Value::Null
        } else {
            serde_json::from_slice(data)?
        };
        match value {
            Value::Null => Ok((Self::default(), false)),
            Value::Array(items) => Ok((
                Self {
                    hosts: items.into_iter().map(Host::from_value).collect(),
                    ..Self::default()
                },
                true,
            )),
            Value::Object(mut m) => {
                let version = take_u64(&mut m, "version").max(HOSTS_VERSION);
                let folders = m.remove("folders").and_then(|v| v.as_array().cloned()).unwrap_or_default().into_iter().map(Folder::from_value).collect();
                let hosts = m.remove("hosts").and_then(|v| v.as_array().cloned()).unwrap_or_default().into_iter().map(Host::from_value).collect();
                Ok((Self { version, folders, hosts, extra: m }, false))
            }
            _ => Err(ConfigError::InvalidFormat(
                "hosts.json must be an array or object".into(),
            )),
        }
    }
    pub fn to_value(&self) -> Value {
        let mut m = self.extra.clone();
        m.insert(
            "version".into(),
            Value::from(self.version.max(HOSTS_VERSION)),
        );
        m.insert(
            "folders".into(),
            Value::Array(self.folders.iter().map(Folder::to_value).collect()),
        );
        m.insert(
            "hosts".into(),
            Value::Array(self.hosts.iter().map(Host::to_value).collect()),
        );
        Value::Object(m)
    }
    pub fn host(&self, id: &str) -> Option<&Host> {
        self.hosts.iter().find(|h| h.id == id)
    }
    pub fn host_mut(&mut self, id: &str) -> Option<&mut Host> {
        self.hosts.iter_mut().find(|h| h.id == id)
    }
    pub fn add(&mut self, mut host: Host) {
        if host.id.is_empty() { host.id = format!("h{}", now_millis()); }
        self.hosts.push(host);
    }
    pub fn live(&self) -> Vec<&Host> { self.hosts.iter().filter(|h| !h.deleted()).collect() }
    pub fn mark_connected(&mut self, id: &str, timestamp: impl Into<String>) -> bool {
        if let Some(h) = self.host_mut(id) { h.last_connected_at = timestamp.into(); true } else { false }
    }
    pub fn soft_delete(&mut self, id: &str, timestamp: impl Into<String>) -> bool {
        if let Some(h) = self.host_mut(id) { if !h.deleted() { h.deleted_at = timestamp.into(); } true } else { false }
    }
    pub fn restore(&mut self, id: &str) -> bool {
        if let Some(h) = self.host_mut(id) { h.deleted_at.clear(); true } else { false }
    }
    pub fn load(path: impl AsRef<Path>) -> Result<Self, ConfigError> {
        let path = path.as_ref();
        let data = match fs::read(path) {
            Ok(v) => v,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(e) => return Err(e.into()),
        };
        match Self::parse(&data) {
            Ok((store, migrated)) => {
                if migrated {
                    let backup = path.parent().unwrap_or(Path::new(".")).join(V1_BACKUP_NAME);
                    if !backup.exists() {
                        let _ = write_atomic(&backup, &data);
                    }
                }
                Ok(store)
            }
            Err(err) => {
                let broken = path.with_extension(format!("json.broken-{}", now_millis()));
                let _ = write_atomic(&broken, &data);
                Err(ConfigError::Io(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!(
                        "{}: {err}; preserved as {}",
                        path.display(),
                        broken.display()
                    ),
                )))
            }
        }
    }
    pub fn save(&self, path: impl AsRef<Path>) -> Result<(), ConfigError> {
        let mut bytes = serde_json::to_vec_pretty(&self.to_value())?;
        bytes.push(b'\n');
        write_atomic(path.as_ref(), &bytes)?;
        Ok(())
    }
}

fn now_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}
fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    let id = TEMP_ID.fetch_add(1, Ordering::Relaxed);
    let temp: PathBuf = parent.join(format!(".finalshell-{}-{id}.tmp", std::process::id()));
    let mut file: File = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)?;
    let result = (|| {
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        #[cfg(windows)]
        if path.exists() {
            fs::remove_file(path)?;
        }
        fs::rename(&temp, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_legacy_array_and_writes_v2() {
        let old = br#"[{"id":"h1","addr":"example.com","user":"root","auth":"key","futureHostField":{"x":1}}]"#;
        let (mut store, migrated) = Store::parse(old).unwrap();
        assert!(migrated);
        assert_eq!(store.hosts[0].target(), "root@example.com");
        store.hosts[0].name = "prod".into();
        let value = store.to_value();
        assert_eq!(value["version"], 2);
        assert_eq!(value["hosts"][0]["futureHostField"]["x"], 1);
        assert_eq!(value["hosts"][0]["name"], "prod");
    }

    #[test]
    fn round_trips_proxy_tunnels_and_unknown_nested_fields() {
        let input = br#"{"version":3,"theme":"dark","folders":[],"hosts":[{"id":"h1","addr":"db","proxy":{"kind":"jump","jumpHost":"h0","vendorFlag":true},"tunnels":[{"kind":"local","listenPort":8080,"targetHost":"localhost","targetPort":80,"vendorTunnel":"x"}],"vendorHost":42}]}"#;
        let (store, migrated) = Store::parse(input).unwrap();
        assert!(!migrated);
        let value = store.to_value();
        assert_eq!(value["version"], 3);
        assert_eq!(value["theme"], "dark");
        assert_eq!(value["hosts"][0]["proxy"]["vendorFlag"], true);
        assert_eq!(value["hosts"][0]["tunnels"][0]["vendorTunnel"], "x");
        assert_eq!(value["hosts"][0]["vendorHost"], 42);
    }

    #[test]
    fn host_lifecycle_keeps_soft_deleted_entries() {
        let mut store = Store::default();
        store.add(Host {
            id: "h1".into(),
            addr: "host".into(),
            ..Host::default()
        });
        assert!(store.mark_connected("h1", "2026-11-25T10:00:00Z"));
        assert!(store.soft_delete("h1", "2026-11-25T11:00:00Z"));
        assert!(store.live().is_empty());
        assert!(store.restore("h1"));
        assert_eq!(store.live().len(), 1);
        assert_eq!(
            store.host("h1").unwrap().last_connected_at,
            "2026-11-25T10:00:00Z"
        );
    }

    #[test]
    fn saves_and_loads_atomically_from_disk() {
        let dir = std::env::temp_dir().join(format!("finalshell-config-{}", now_millis()));
        let path = dir.join("hosts.json");
        let mut store = Store::default();
        store.add(Host {
            id: "h1".into(),
            addr: "example.com".into(),
            user: "root".into(),
            ..Host::default()
        });
        store.save(&path).unwrap();
        let loaded = Store::load(&path).unwrap();
        assert_eq!(loaded.hosts[0].target(), "root@example.com");
        fs::remove_dir_all(dir).unwrap();
    }
}
