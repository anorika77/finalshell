use std::{collections::BTreeSet, fs, path::Path};

use crate::config::{Host, AUTH_KEY, AUTH_PASSWORD};

#[derive(Debug, Clone, PartialEq, Eq)]
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

pub fn read_ssh_config(path: impl AsRef<Path>, local_user: &str) -> Result<Vec<SshConfigEntry>, ImportError> {
    let path = path.as_ref();
    let text = fs::read_to_string(path).map_err(|e| ImportError(format!("{}: {e}", path.display())))?;
    parse_ssh_config(&text, local_user)
}

/// Parse the useful connection settings from OpenSSH client configuration.
/// Include and Match blocks are deliberately not expanded; no secret is imported.
pub fn parse_ssh_config(text: &str, local_user: &str) -> Result<Vec<SshConfigEntry>, ImportError> {
    #[derive(Default, Clone)]
    struct Settings {
        hostname: Option<String>, port: Option<u16>, user: Option<String>,
        identity: Option<String>, proxy_jump: Option<String>,
    }
    let mut aliases = Vec::<String>::new();
    let mut lines = Vec::<(String, String, Vec<String>)>::new();
    let mut patterns: Vec<String> = Vec::new();
    let mut in_match = false;
    for (index, raw) in text.lines().enumerate() {
        let (key, args) = fields(raw).map_err(|e| ImportError(format!("line {}: {e}", index + 1)))?;
        if key.is_empty() { continue; }
        match key.as_str() {
            "host" => {
                in_match = false;
                patterns = args.clone();
                for name in args {
                    if !name.chars().any(|ch| matches!(ch, '*' | '?' | '!')) && !aliases.iter().any(|a| a.eq_ignore_ascii_case(&name)) {
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
        return Err(ImportError("SSH config contains no named Host entries".into()));
    }
    let mut out = Vec::new();
    for alias in aliases {
        let mut s = Settings::default();
        for (key, value, conds) in &lines {
            if !matches_host(&alias, conds) { continue; }
            match key.as_str() {
                "hostname" if s.hostname.is_none() => s.hostname = Some(value.clone()),
                "port" if s.port.is_none() => {
                    if let Ok(p) = value.parse::<u16>() { if p > 0 { s.port = Some(p); } }
                }
                "user" if s.user.is_none() => s.user = Some(value.clone()),
                "identityfile" if s.identity.is_none() => s.identity = Some(value.clone()),
                "proxyjump" if s.proxy_jump.is_none() => s.proxy_jump = Some(value.clone()),
                _ => {}
            }
        }
        let addr = s.hostname.map(|v| expand_identity(&v, &alias, &alias, local_user, local_user, s.port.unwrap_or(22))).unwrap_or_else(|| alias.clone());
        let user = s.user.filter(|v| !v.is_empty()).unwrap_or_else(|| local_user.to_owned());
        let mut host = Host {
            name: alias.clone(), addr: addr.clone(), user: user.clone(),
            port: s.port.unwrap_or(22), auth: AUTH_PASSWORD.into(),
            ..Host::default()
        };
        if let Some(identity) = s.identity.filter(|v| !v.eq_ignore_ascii_case("none")) {
            host.auth = AUTH_KEY.into();
            host.key_path = expand_identity(&identity, &alias, &addr, &user, local_user, host.port);
        }
        let proxy_jump = s.proxy_jump.filter(|v| !v.eq_ignore_ascii_case("none"))
            .map(|v| v.split(',').last().unwrap_or("").trim().to_owned())
            .filter(|v| !v.is_empty());
        out.push(SshConfigEntry { host, proxy_jump });
    }
    // Stable order and case-insensitive uniqueness are important when jump aliases are resolved.
    let mut seen = BTreeSet::new();
    out.retain(|e| seen.insert(e.host.name.to_ascii_lowercase()));
    Ok(out)
}

fn expand_identity(value: &str, alias: &str, addr: &str, user: &str, local_user: &str, port: u16) -> String {
    let home = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")).unwrap_or_default();
    let mut path = value.replace("%d", &home).replace("%u", local_user)
        .replace("%h", addr).replace("%r", user).replace("%n", alias).replace("%p", &port.to_string());
    if path == "~" { path = home; }
    else if let Some(rest) = path.strip_prefix("~/") { path = Path::new(&home).join(rest).to_string_lossy().into_owned(); }
    path.replace("%%", "%")
}

fn matches_host(name: &str, patterns: &[String]) -> bool {
    if patterns.is_empty() { return true; }
    let mut positive = false;
    for pattern in patterns {
        let (negative, p) = pattern.strip_prefix('!').map(|p| (true, p)).unwrap_or((false, pattern.as_str()));
        if glob_match(&p.to_ascii_lowercase(), &name.to_ascii_lowercase()) {
            if negative { return false; }
            positive = true;
        }
    }
    positive
}

fn glob_match(pattern: &str, value: &str) -> bool {
    let (p, s) = (pattern.as_bytes(), value.as_bytes());
    let (mut i, mut j, mut star, mut retry) = (0, 0, None, 0);
    while j < s.len() {
        if i < p.len() && (p[i] == b'?' || p[i] == s[j]) { i += 1; j += 1; }
        else if i < p.len() && p[i] == b'*' { star = Some(i); i += 1; retry = j; }
        else if let Some(pos) = star { retry += 1; i = pos + 1; j = retry; }
        else { return false; }
    }
    while i < p.len() && p[i] == b'*' { i += 1; }
    i == p.len()
}

fn fields(line: &str) -> Result<(String, Vec<String>), String> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') { return Ok((String::new(), Vec::new())); }
    let split = line.find([' ', '\t', '=']).unwrap_or(line.len());
    let key = line[..split].to_ascii_lowercase();
    let mut rest = line[split..].trim_start();
    if let Some(r) = rest.strip_prefix('=') { rest = r.trim_start(); }
    let mut args = Vec::new();
    let mut value = String::new();
    let mut quote = None;
    let mut started = false;
    let mut chars = rest.chars().peekable();
    while let Some(c) = chars.next() {
        if let Some(q) = quote {
            if c == q { quote = None; }
            else if c == '\\' && chars.peek().is_some_and(|n| *n == q || *n == '\\') {
                value.push(chars.next().unwrap());
            } else { value.push(c); }
        } else {
            match c {
                '"' | '\'' => { quote = Some(c); started = true; }
                ' ' | '\t' => if started { args.push(std::mem::take(&mut value)); started = false; },
                '#' if !started => break,
                '\\' if chars.peek().is_some_and(|n| *n == ' ' || *n == '"' || *n == '\'') => {
                    value.push(chars.next().unwrap()); started = true;
                }
                _ => { value.push(c); started = true; }
            }
        }
    }
    if quote.is_some() { return Err("unclosed quote".into()); }
    if started { args.push(value); }
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
        assert!(parse_ssh_config("Host a\n User \\"oops\n", "u").is_err());
    }
}
