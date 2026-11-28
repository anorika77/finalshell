use std::{collections::BTreeSet, fs, path::Path};

use crate::config::{AUTH_KEY, AUTH_PASSWORD, Host};

#[derive(Debug, Clone, PartialEq)]
pub struct SshConfigEntry {
    pub host: Host,
    pub proxy_jump: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportError(pub String);

impl std::fmt::Display for ImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for ImportError {}

pub fn read_ssh_config(
    path: impl AsRef<Path>,
    local_user: &str,
) -> Result<Vec<SshConfigEntry>, ImportError> {
    let path = path.as_ref();
    let text =
        fs::read_to_string(path).map_err(|e| ImportError(format!("{}: {e}", path.display())))?;
    parse_ssh_config(&text, local_user)
}

/// Parse useful connection settings from OpenSSH client configuration.
/// Include and Match blocks are not expanded; no secret is imported.
pub fn parse_ssh_config(text: &str, local_user: &str) -> Result<Vec<SshConfigEntry>, ImportError> {
    #[derive(Default, Clone)]
    struct Settings {
        hostname: Option<String>,
        port: Option<u16>,
        user: Option<String>,
        identity: Option<String>,
        proxy_jump: Option<String>,
    }

    let mut aliases = Vec::<String>::new();
    let mut lines = Vec::<(String, String, Vec<String>)>::new();
    let mut patterns: Vec<String> = Vec::new();
    let mut in_match = false;

    for (index, raw) in text.lines().enumerate() {
        let (key, args) =
            fields(raw).map_err(|e| ImportError(format!("line {}: {e}", index + 1)))?;
        if key.is_empty() {
            continue;
        }
        match key.as_str() {
            "host" => {
                in_match = false;
                patterns = args.clone();
                for name in args {
                    if !name.chars().any(|ch| matches!(ch, '*' | '?' | '!'))
                        && !aliases.iter().any(|a| a.eq_ignore_ascii_case(&name))
                    {
                        aliases.push(name);
                    }
                }
            }
            "match" => in_match = true,
            _ if in_match => {}
            "hostname" | "port" | "user" | "identityfile" | "proxyjump" => {
                if let Some(value) = args.first() {
                    lines.push((key, value.clone(), patterns.clone()));
                }
            }
            _ => {}
        }
    }

    if aliases.is_empty() {
        return Err(ImportError(
            "SSH config contains no named Host entries".into(),
        ));
    }

    let mut out = Vec::new();
    for alias in aliases {
        let mut settings = Settings::default();
        for (key, value, conditions) in &lines {
            if !matches_host(&alias, conditions) {
                continue;
            }
            match key.as_str() {
                "hostname" if settings.hostname.is_none() => {
                    settings.hostname = Some(value.clone());
                }
                "port" if settings.port.is_none() => {
                    if let Ok(port) = value.parse::<u16>() {
                        if port > 0 {
                            settings.port = Some(port);
                        }
                    }
                }
                "user" if settings.user.is_none() => {
                    settings.user = Some(value.clone());
                }
                "identityfile" if settings.identity.is_none() => {
                    settings.identity = Some(value.clone());
                }
                "proxyjump" if settings.proxy_jump.is_none() => {
                    settings.proxy_jump = Some(value.clone());
                }
                _ => {}
            }
        }

        let port = settings.port.unwrap_or(22);
        let addr = settings
            .hostname
            .map(|value| expand_identity(&value, &alias, &alias, local_user, local_user, port))
            .unwrap_or_else(|| alias.clone());
        let user = settings
            .user
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| local_user.to_owned());
        let mut host = Host {
            name: alias.clone(),
            addr: addr.clone(),
            user: user.clone(),
            port,
            auth: AUTH_PASSWORD.into(),
            ..Host::default()
        };

        if let Some(identity) = settings
            .identity
            .filter(|value| !value.eq_ignore_ascii_case("none"))
        {
            host.auth = AUTH_KEY.into();
            host.key_path = expand_identity(&identity, &alias, &addr, &user, local_user, port);
        }

        let proxy_jump = settings
            .proxy_jump
            .filter(|value| !value.eq_ignore_ascii_case("none"))
            .map(|value| value.split(',').last().unwrap_or("").trim().to_owned())
            .filter(|value| !value.is_empty());
        out.push(SshConfigEntry { host, proxy_jump });
    }

    let mut seen = BTreeSet::new();
    out.retain(|entry| seen.insert(entry.host.name.to_ascii_lowercase()));
    Ok(out)
}

fn expand_identity(
    value: &str,
    alias: &str,
    addr: &str,
    user: &str,
    local_user: &str,
    port: u16,
) -> String {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_default();
    let mut path = value
        .replace("%d", &home)
        .replace("%u", local_user)
        .replace("%h", addr)
        .replace("%r", user)
        .replace("%n", alias)
        .replace("%p", &port.to_string());
    if path == "~" {
        path = home;
    } else if let Some(rest) = path.strip_prefix("~/") {
        path = Path::new(&home).join(rest).to_string_lossy().into_owned();
    }
    path.replace("%%", "%")
}

fn matches_host(name: &str, patterns: &[String]) -> bool {
    if patterns.is_empty() {
        return true;
    }
    let mut positive = false;
    for pattern in patterns {
        let (negative, pattern) = pattern
            .strip_prefix('!')
            .map(|value| (true, value))
            .unwrap_or((false, pattern.as_str()));
        if glob_match(&pattern.to_ascii_lowercase(), &name.to_ascii_lowercase()) {
            if negative {
                return false;
            }
            positive = true;
        }
    }
    positive
}

fn glob_match(pattern: &str, value: &str) -> bool {
    let (pattern, value) = (pattern.as_bytes(), value.as_bytes());
    let (mut i, mut j, mut star, mut retry) = (0, 0, None, 0);
    while j < value.len() {
        if i < pattern.len() && (pattern[i] == b'?' || pattern[i] == value[j]) {
            i += 1;
            j += 1;
        } else if i < pattern.len() && pattern[i] == b'*' {
            star = Some(i);
            i += 1;
            retry = j;
        } else if let Some(position) = star {
            retry += 1;
            i = position + 1;
            j = retry;
        } else {
            return false;
        }
    }
    while i < pattern.len() && pattern[i] == b'*' {
        i += 1;
    }
    i == pattern.len()
}

fn fields(line: &str) -> Result<(String, Vec<String>), String> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return Ok((String::new(), Vec::new()));
    }
    let split = line.find([' ', '\t', '=']).unwrap_or(line.len());
    let key = line[..split].to_ascii_lowercase();
    let mut rest = line[split..].trim_start();
    if let Some(value) = rest.strip_prefix('=') {
        rest = value.trim_start();
    }

    let mut args = Vec::new();
    let mut value = String::new();
    let mut quote = None;
    let mut started = false;
    let mut chars = rest.chars().peekable();
    while let Some(ch) = chars.next() {
        if let Some(q) = quote {
            if ch == q {
                quote = None;
            } else if ch == '\\' && chars.peek().is_some_and(|next| *next == q || *next == '\\') {
                value.push(chars.next().unwrap());
            } else {
                value.push(ch);
            }
        } else {
            match ch {
                '"' | '\'' => {
                    quote = Some(ch);
                    started = true;
                }
                ' ' | '\t' => {
                    if started {
                        args.push(std::mem::take(&mut value));
                        started = false;
                    }
                }
                '#' if !started => break,
                '\\' if chars
                    .peek()
                    .is_some_and(|next| *next == ' ' || *next == '"' || *next == '\'') =>
                {
                    value.push(chars.next().unwrap());
                    started = true;
                }
                _ => {
                    value.push(ch);
                    started = true;
                }
            }
        }
    }
    if quote.is_some() {
        return Err("unclosed quote".into());
    }
    if started {
        args.push(value);
    }
    Ok((key, args))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_hosts_globals_patterns_and_first_value_wins() {
        let input = r#"
            User global
            Port 2200
            Host *
              IdentityFile ~/.ssh/id_ed25519
            Host prod-*
              HostName %h.internal
              User deploy
              Port 2222
            Host prod-db !prod-db-old
              HostName db.internal
              ProxyJump bastion
            Host prod-db-old
              HostName old.internal
        "#;
        let entries = parse_ssh_config(input, "local").unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].host.name, "prod-db");
        assert_eq!(entries[0].host.addr, "prod-db.internal");
        assert_eq!(entries[0].host.port, 2200);
        assert_eq!(entries[0].host.user, "global");
        assert_eq!(entries[0].proxy_jump.as_deref(), Some("bastion"));
    }

    #[test]
    fn imports_alias_and_jump_without_importing_passwords() {
        let input = "Host bastion\n HostName bastion.example\n User ops\nHost app\n HostName app.example\n ProxyJump bastion\n";
        let entries = parse_ssh_config(input, "local").unwrap();
        assert_eq!(entries[0].host.addr, "bastion.example");
        assert_eq!(entries[1].proxy_jump.as_deref(), Some("bastion"));
        assert_eq!(entries[1].host.auth, AUTH_PASSWORD);
    }

    #[test]
    fn rejects_configs_without_named_hosts_and_bad_quotes() {
        assert!(parse_ssh_config("Host *\n User root\n", "u").is_err());
        assert!(parse_ssh_config("Host a\n User \"oops\n", "u").is_err());
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct FinalShellEntry {
    pub host: Host,
    pub folders: Vec<String>,
    pub needs_key: bool,
}

/// Read FinalShell's native data directory or an exported connection folder.
/// Encrypted passwords and private-key material are deliberately never imported.
pub fn read_finalshell_dir(path: impl AsRef<Path>) -> Result<Vec<FinalShellEntry>, ImportError> {
    use serde_json::Value;
    let root = path.as_ref();
    let conn = if root.join("conn").is_dir() {
        root.join("conn")
    } else {
        root.to_path_buf()
    };
    if !conn.is_dir() {
        return Err(ImportError(format!(
            "{} is not a directory",
            conn.display()
        )));
    }
    fn text(v: Option<&Value>) -> String {
        match v {
            Some(Value::String(s)) => s.trim().to_owned(),
            Some(Value::Number(n)) => n.to_string(),
            _ => String::new(),
        }
    }
    fn number(v: Option<&Value>) -> i64 {
        text(v).parse().unwrap_or(0)
    }
    fn walk(
        dir: &Path,
        root: &Path,
        folders: &[String],
        out: &mut Vec<FinalShellEntry>,
        count: &mut usize,
    ) -> Result<(), ImportError> {
        let mut names = folders.to_vec();
        let folder_file = dir.join("folder.json");
        if let Ok(bytes) = fs::read(&folder_file) {
            if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&bytes) {
                if number(v.get("delete_time")) != 0 {
                    return Ok(());
                }
                let name = text(v.get("name"));
                if !name.is_empty() {
                    names.push(name);
                }
            }
        } else if dir != root {
            if let Some(name) = dir.file_name().and_then(|n| n.to_str()) {
                names.push(name.to_owned());
            }
        }
        let entries =
            fs::read_dir(dir).map_err(|e| ImportError(format!("{}: {e}", dir.display())))?;
        for item in entries {
            let item = item.map_err(|e| ImportError(e.to_string()))?;
            let p = item.path();
            if p.is_dir() {
                if names.len() < 16 {
                    walk(&p, root, &names, out, count)?;
                }
                continue;
            }
            if !p
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.to_ascii_lowercase().ends_with("_connect_config.json"))
            {
                continue;
            }
            *count += 1;
            if *count > 10000 {
                return Err(ImportError("FinalShell import exceeds 10000 files".into()));
            }
            let data = fs::read(&p).map_err(|e| ImportError(format!("{}: {e}", p.display())))?;
            if data.len() > 1 << 20 {
                continue;
            }
            let value: Value = match serde_json::from_slice(&data) {
                Ok(v) => v,
                Err(_) => continue,
            };
            let rows: Vec<Value> = value.as_array().cloned().unwrap_or_else(|| vec![value]);
            for row in rows {
                if number(row.get("delete_time")) != 0 {
                    continue;
                }
                let typ = number(row.get("conection_type"));
                if typ != 0 && typ != 100 {
                    continue;
                }
                let addr = text(row.get("host"));
                if addr.is_empty() {
                    continue;
                }
                let auth_type = number(row.get("authentication_type"));
                let needs_key = auth_type == 2
                    || (auth_type != 1 && !text(row.get("secret_key_id")).is_empty());
                let port = number(row.get("port"));
                let host = Host {
                    name: text(row.get("name")),
                    addr,
                    port: if (1..=65535).contains(&port) && port != 22 {
                        port as u16
                    } else {
                        0
                    },
                    user: text(row.get("user_name")),
                    auth: if needs_key {
                        AUTH_KEY.into()
                    } else {
                        AUTH_PASSWORD.into()
                    },
                    notes: text(row.get("description")),
                    ..Host::default()
                };
                out.push(FinalShellEntry {
                    host,
                    folders: names.clone(),
                    needs_key,
                });
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    let mut count = 0;
    walk(&conn, &conn, &[], &mut out, &mut count)?;
    if out.is_empty() {
        return Err(ImportError(
            "no SSH connections found in FinalShell data".into(),
        ));
    }
    Ok(out)
}

/// Read Xshell session files (.xsh) from a Sessions directory or one file.
/// Passwords and Xshell's private key store are intentionally not imported.
pub fn read_xshell(path: impl AsRef<Path>) -> Result<Vec<FinalShellEntry>, ImportError> {
    fn decode(bytes: &[u8]) -> String {
        if bytes.starts_with(&[0xff, 0xfe]) {
            let words = bytes[2..]
                .chunks_exact(2)
                .map(|v| u16::from_le_bytes([v[0], v[1]]))
                .collect::<Vec<_>>();
            return String::from_utf16_lossy(&words);
        }
        if bytes.starts_with(&[0xfe, 0xff]) {
            let words = bytes[2..]
                .chunks_exact(2)
                .map(|v| u16::from_be_bytes([v[0], v[1]]))
                .collect::<Vec<_>>();
            return String::from_utf16_lossy(&words);
        }
        if let Ok(s) = std::str::from_utf8(bytes) {
            return s.to_owned();
        }
        encoding_rs::GBK.decode(bytes).0.into_owned()
    }

    fn parse(name: String, folders: Vec<String>, bytes: &[u8]) -> Option<FinalShellEntry> {
        let text = decode(bytes);
        let mut section = String::new();
        let mut values = std::collections::BTreeMap::<String, String>::new();
        for line in text.lines().map(str::trim) {
            if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
                continue;
            }
            if line.starts_with('[') && line.ends_with(']') {
                section = line[1..line.len() - 1].trim().to_ascii_lowercase();
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let key = key.trim().to_ascii_lowercase();
            if matches!(
                section.as_str(),
                "connection" | "connection:authentication" | "connection:proxy"
            ) && matches!(
                key.as_str(),
                "host"
                    | "port"
                    | "protocol"
                    | "description"
                    | "username"
                    | "method"
                    | "userkey"
                    | "proxy"
                    | "firewall"
            ) {
                values.insert(format!("{section}:{key}"), value.trim().to_owned());
            }
        }
        let protocol = values
            .get("connection:protocol")
            .map(String::as_str)
            .unwrap_or("SSH");
        if !protocol.to_ascii_uppercase().starts_with("SSH") {
            return None;
        }
        let addr = values.get("connection:host")?.trim().to_owned();
        if addr.is_empty() {
            return None;
        }
        let method = values
            .get("connection:authentication:method")
            .map(|s| s.to_ascii_uppercase())
            .unwrap_or_default();
        let user_key = values
            .get("connection:authentication:userkey")
            .is_some_and(|s| !s.is_empty());
        let needs_key =
            method == "1" || method.contains("PUBLICKEY") || (method.is_empty() && user_key);
        let port = values
            .get("connection:port")
            .and_then(|s| s.parse::<u16>().ok())
            .filter(|p| *p > 0 && *p != 22)
            .unwrap_or(0);
        let host = Host {
            name,
            addr,
            port,
            user: values
                .get("connection:authentication:username")
                .cloned()
                .unwrap_or_default(),
            auth: if needs_key {
                AUTH_KEY.into()
            } else {
                AUTH_PASSWORD.into()
            },
            notes: values
                .get("connection:description")
                .cloned()
                .unwrap_or_default(),
            ..Host::default()
        };
        Some(FinalShellEntry {
            host,
            folders,
            needs_key,
        })
    }

    fn walk(
        dir: &Path,
        folders: &[String],
        out: &mut Vec<FinalShellEntry>,
        count: &mut usize,
    ) -> Result<(), ImportError> {
        for item in fs::read_dir(dir).map_err(|e| ImportError(format!("{}: {e}", dir.display())))? {
            let item = item.map_err(|e| ImportError(e.to_string()))?;
            let path = item.path();
            if path.is_dir() {
                if folders.len() < 16 {
                    let mut next = folders.to_vec();
                    next.push(item.file_name().to_string_lossy().into_owned());
                    walk(&path, &next, out, count)?;
                }
                continue;
            }
            if !path
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| ext.eq_ignore_ascii_case("xsh"))
            {
                continue;
            }
            *count += 1;
            if *count > 10000 {
                return Err(ImportError("Xshell import exceeds 10000 files".into()));
            }
            let bytes =
                fs::read(&path).map_err(|e| ImportError(format!("{}: {e}", path.display())))?;
            if bytes.len() > 1 << 20 {
                continue;
            }
            let name = path
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            if let Some(entry) = parse(name, folders.to_vec(), &bytes) {
                out.push(entry);
            }
        }
        Ok(())
    }

    let path = path.as_ref();
    let mut out = Vec::new();
    let mut count = 0;
    if path.is_file() {
        let bytes = fs::read(path).map_err(|e| ImportError(format!("{}: {e}", path.display())))?;
        let name = path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        if let Some(entry) = parse(name, Vec::new(), &bytes) {
            out.push(entry);
        }
    } else {
        let root = if path.join("Sessions").is_dir() {
            path.join("Sessions")
        } else {
            path.to_path_buf()
        };
        walk(&root, &[], &mut out, &mut count)?;
    }
    if out.is_empty() {
        return Err(ImportError("no SSH sessions found in Xshell data".into()));
    }
    Ok(out)
}

/// Read MobaXterm INI or exported session files; encrypted passwords are skipped.
pub fn read_mobaxterm(path: impl AsRef<Path>) -> Result<Vec<FinalShellEntry>, ImportError> {
    let path = path.as_ref();
    let bytes = fs::read(path).map_err(|e| ImportError(format!("{}: {e}", path.display())))?;
    if bytes.len() > 16 * 1024 * 1024 {
        return Err(ImportError("MobaXterm file exceeds 16 MiB".into()));
    }
    let text = if let Ok(s) = std::str::from_utf8(&bytes) {
        s.to_owned()
    } else {
        encoding_rs::GBK.decode(&bytes).0.into_owned()
    };
    fn unescape(v: &str) -> String {
        v.replace("__PTVIRG__", ";")
            .replace("__DBLQUO__", "\"")
            .replace("__PIPE__", "|")
            .replace("__PERCENT__", "%")
            .replace("__DIEZE__", "#")
    }
    let mut out = Vec::new();
    let mut folder = Vec::<String>::new();
    let mut bookmarks = false;
    let mut header = true;
    let mut pending = Vec::<(String, String)>::new();
    let flush = |pending: &mut Vec<(String, String)>,
                 folders: &[String],
                 out: &mut Vec<FinalShellEntry>| {
        for (name, value) in pending.drain(..) {
            let parts = value.split('#').collect::<Vec<_>>();
            if parts.len() < 3 {
                continue;
            }
            let fields = parts[2].split('%').map(unescape).collect::<Vec<_>>();
            let field = |n: usize| fields.get(n).map(String::as_str).unwrap_or("");
            if (!field(0).is_empty() && field(0) != "0") || field(1).trim().is_empty() {
                continue;
            }
            let port = field(2)
                .parse::<u16>()
                .ok()
                .filter(|p| *p > 0 && *p != 22)
                .unwrap_or(0);
            let key = field(14).replace("_CurrentDrive_", "C");
            let needs_key = !key.is_empty();
            let host = Host {
                name: unescape(&name).trim().to_owned(),
                addr: field(1).trim().to_owned(),
                port,
                user: if field(3) == "<default>" {
                    String::new()
                } else {
                    field(3).to_owned()
                },
                auth: if needs_key {
                    AUTH_KEY.into()
                } else {
                    AUTH_PASSWORD.into()
                },
                key_path: if key.starts_with(r"\\") || (key.len() > 2 && key.as_bytes()[1] == b':')
                {
                    key
                } else {
                    String::new()
                },
                notes: parts
                    .get(5)
                    .map(|v| unescape(v).trim().to_owned())
                    .unwrap_or_default(),
                ..Host::default()
            };
            out.push(FinalShellEntry {
                host,
                folders: folders.to_vec(),
                needs_key,
            });
        }
    };
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with(';') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            flush(&mut pending, &folder, &mut out);
            let section = line[1..line.len() - 1].trim().to_ascii_lowercase();
            bookmarks = section == "bookmarks"
                || section
                    .strip_prefix("bookmarks_")
                    .is_some_and(|n| !n.is_empty() && n.parse::<u32>().is_ok());
            header = false;
            folder.clear();
            continue;
        }
        if !bookmarks && !header {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if bookmarks && key.trim().eq_ignore_ascii_case("SubRep") {
            folder = value
                .split('\\')
                .map(|s| unescape(s).trim().to_owned())
                .filter(|s| !s.is_empty())
                .collect();
            continue;
        }
        if bookmarks && key.trim().eq_ignore_ascii_case("ImgNum") {
            continue;
        }
        if header && value.matches('#').count() < 2 {
            continue;
        }
        pending.push((key.trim().to_owned(), value.trim().to_owned()));
    }
    flush(&mut pending, &folder, &mut out);
    if out.is_empty() {
        return Err(ImportError(
            "no SSH sessions found in MobaXterm file".into(),
        ));
    }
    Ok(out)
}
