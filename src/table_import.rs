use crate::config::{AUTH_KEY, AUTH_PASSWORD, Host};
use calamine::{Reader, open_workbook_auto_from_rs};
use csv::ReaderBuilder;
use std::{collections::BTreeMap, fs, io::Cursor, path::Path};

const MAX_FILE_SIZE: u64 = 10 * 1024 * 1024;
const MAX_ROWS: usize = 10_000;
const HEADER_ROWS: usize = 10;

#[derive(Debug, Clone, PartialEq)]
pub struct TableEntry {
    pub host: Host,
    pub folders: Vec<String>,
    pub key_file: String,
    /// Return this to the caller for OS credential-store insertion; never persist it.
    pub password: Option<String>,
    pub row: usize,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableProblem {
    pub row: usize,
    pub reason: &'static str,
    pub detail: String,
}
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TableImport {
    pub entries: Vec<TableEntry>,
    pub problems: Vec<TableProblem>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Column {
    Name,
    Host,
    Port,
    User,
    Password,
    Key,
    Folder,
    Notes,
}

const HEADERS: &[(Column, &[&str])] = &[
    (
        Column::Name,
        &["name", "label", "title", "名称", "名字", "别名"],
    ),
    (
        Column::Host,
        &[
            "host",
            "hostname",
            "address",
            "ip",
            "ipaddress",
            "主机",
            "主机名",
            "主机地址",
            "地址",
            "ip地址",
        ],
    ),
    (Column::Port, &["port", "端口"]),
    (
        Column::User,
        &["user", "username", "login", "用户", "用户名", "账号"],
    ),
    (Column::Password, &["password", "密码"]),
    (
        Column::Key,
        &[
            "key",
            "privatekey",
            "keyfile",
            "identityfile",
            "私钥",
            "私钥文件",
        ],
    ),
    (Column::Folder, &["folder", "group", "文件夹", "分组"]),
    (
        Column::Notes,
        &[
            "notes",
            "note",
            "description",
            "comment",
            "comments",
            "备注",
            "说明",
        ],
    ),
];

fn column_of(header: &str) -> Option<Column> {
    let key: String = header
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '_' && *c != '-')
        .flat_map(char::to_lowercase)
        .collect();
    HEADERS
        .iter()
        .find_map(|(column, names)| names.contains(&key.as_str()).then_some(*column))
}
fn columns(cells: &[String]) -> Option<BTreeMap<Column, usize>> {
    let mut out = BTreeMap::new();
    for (i, cell) in cells.iter().enumerate() {
        if let Some(c) = column_of(cell) {
            out.entry(c).or_insert(i);
        }
    }
    out.contains_key(&Column::Host).then_some(out)
}
fn empty_row(cells: &[String]) -> bool {
    cells.iter().all(|c| c.trim().is_empty())
}

fn decode_text(data: &[u8]) -> String {
    if data.starts_with(&[0xef, 0xbb, 0xbf]) {
        return String::from_utf8_lossy(&data[3..]).into_owned();
    }
    if data.starts_with(&[0xff, 0xfe]) {
        let u = data[2..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| u16::from_le_bytes(*b))
            .collect::<Vec<_>>();
        return String::from_utf16_lossy(&u);
    }
    if data.starts_with(&[0xfe, 0xff]) {
        let u = data[2..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| u16::from_be_bytes(*b))
            .collect::<Vec<_>>();
        return String::from_utf16_lossy(&u);
    }
    if let Ok(s) = std::str::from_utf8(data) {
        return s.to_owned();
    }
    encoding_rs::GB18030.decode(data).0.into_owned()
}
fn records(text: &str, delimiter: u8) -> Result<Vec<(usize, Vec<String>)>, String> {
    let mut reader = ReaderBuilder::new()
        .delimiter(delimiter)
        .has_headers(false)
        .flexible(true)
        .from_reader(text.as_bytes());
    let mut out = Vec::new();
    for item in reader.records() {
        let rec = item.map_err(|e| e.to_string())?;
        let line = rec
            .position()
            .map(|p| p.line() as usize)
            .unwrap_or(out.len() + 1);
        out.push((line, rec.iter().map(str::to_owned).collect()));
    }
    Ok(out)
}
type TableRows = Vec<(usize, Vec<String>)>;

fn detect_delimiter(text: &str) -> Result<(u8, TableRows), String> {
    let first = text.lines().next().unwrap_or("").trim();
    if first
        .get(..4)
        .is_some_and(|s| s.eq_ignore_ascii_case("sep="))
    {
        let value = &first[4..];
        if value.chars().count() == 1 {
            let d = value.as_bytes()[0];
            let rest = text.lines().skip(1).collect::<Vec<_>>().join("\n");
            return Ok((d, records(&rest, d)?));
        }
    }
    let mut best: Option<(usize, u8, TableRows)> = None;
    for d in *b"\t;," {
        let rows = records(text, d)?;
        let mut seen = 0;
        for (_, cells) in &rows {
            if empty_row(cells) {
                continue;
            }
            seen += 1;
            if let Some(cols) = columns(cells) {
                if best
                    .as_ref()
                    .is_none_or(|(score, _, _)| cols.len() > *score)
                {
                    best = Some((cols.len(), d, rows));
                }
                break;
            }
            if seen >= HEADER_ROWS {
                break;
            }
        }
    }
    if let Some((_, d, rows)) = best {
        return Ok((d, rows));
    }
    let line = text.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
    let d = if line.contains('\t') {
        b'\t'
    } else if line.matches(';').count() > line.matches(',').count() {
        b';'
    } else {
        b','
    };
    Ok((d, records(text, d)?))
}
fn folder_path(s: &str) -> Vec<String> {
    s.split(['/', '\\'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .take(16)
        .map(str::to_owned)
        .collect()
}
fn number_text(s: &str) -> String {
    let s = s.trim();
    if s.is_empty()
        || s.chars()
            .any(|c| !c.is_ascii_digit() && !matches!(c, '.' | 'e' | 'E' | '+' | '-'))
    {
        return s.into();
    }
    s.parse::<f64>()
        .ok()
        .filter(|n| n.is_finite())
        .map(|n| {
            if n.fract() == 0.0 {
                format!("{n:.0}")
            } else {
                s.to_owned()
            }
        })
        .unwrap_or_else(|| s.into())
}
fn split_host_port(s: &str) -> (String, Option<String>) {
    let s = s.trim();
    if let Some(rest) = s.strip_prefix('[') {
        if let Some((host, port)) = rest.split_once("]:")
            && !port.is_empty()
            && port.chars().all(|c| c.is_ascii_digit())
        {
            return (host.into(), Some(port.into()));
        }
        return (s.trim_matches(['[', ']']).into(), None);
    }
    if s.matches(':').count() == 1
        && let Some((host, port)) = s.rsplit_once(':')
        && !host.is_empty()
        && !port.is_empty()
        && port.chars().all(|c| c.is_ascii_digit())
    {
        return (host.into(), Some(port.into()));
    }
    (s.into(), None)
}

fn parse_rows(rows: Vec<(usize, Vec<String>)>) -> Result<TableImport, String> {
    let mut result = TableImport::default();
    let mut header: Option<BTreeMap<Column, usize>> = None;
    let mut first_headers = Vec::new();
    let mut before = 0;
    let mut count = 0;
    for (line, cells) in rows {
        if empty_row(&cells) {
            continue;
        }
        if header.is_none() {
            before += 1;
            if first_headers.is_empty() {
                first_headers = cells
                    .iter()
                    .map(|s| s.trim())
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
                    .collect();
            }
            if let Some(found) = columns(&cells) {
                header = Some(found);
                continue;
            }
            if before >= HEADER_ROWS {
                break;
            }
            continue;
        }
        count += 1;
        if count > MAX_ROWS {
            result.problems.push(TableProblem {
                row: line,
                reason: "too-many-rows",
                detail: "only the first 10000 server rows are read".into(),
            });
            break;
        }
        let cols = header.as_ref().expect("header exists");
        let cell = |c: Column| -> String {
            let Some(i) = cols.get(&c) else {
                return String::new();
            };
            let Some(v) = cells.get(*i) else {
                return String::new();
            };
            if c == Column::Password {
                v.clone()
            } else {
                v.trim().into()
            }
        };
        let mut address = cell(Column::Host);
        let mut user = cell(Column::User);
        if let Some((u, h)) = address.split_once('@') {
            if user.is_empty() {
                user = u.into();
            }
            address = h.into();
        }
        let (address, embedded) = split_host_port(&address);
        let port = if cell(Column::Port).is_empty() {
            embedded.unwrap_or_default()
        } else {
            cell(Column::Port)
        };
        if address.is_empty() {
            result.problems.push(TableProblem {
                row: line,
                reason: "no-host",
                detail: "host is empty".into(),
            });
            continue;
        }
        let port = if port.is_empty() {
            Some(22)
        } else {
            number_text(&port).parse::<u16>().ok().filter(|p| *p > 0)
        };
        let Some(port) = port else {
            result.problems.push(TableProblem {
                row: line,
                reason: "bad-port",
                detail: cell(Column::Port),
            });
            continue;
        };
        let key_file = cell(Column::Key);
        let password = if key_file.is_empty() {
            let p = cell(Column::Password);
            (!p.trim().is_empty()).then_some(p)
        } else {
            None
        };
        let host = Host {
            name: cell(Column::Name),
            addr: address.trim_matches(['[', ']']).into(),
            port: if port == 22 { 0 } else { port },
            user,
            auth: if key_file.is_empty() {
                AUTH_PASSWORD.into()
            } else {
                AUTH_KEY.into()
            },
            key_path: key_file.clone(),
            notes: cell(Column::Notes),
            ..Host::default()
        };
        result.entries.push(TableEntry {
            host,
            folders: folder_path(&cell(Column::Folder)),
            key_file,
            password,
            row: line,
        });
    }
    if header.is_none() {
        return Err(format!(
            "no host column; headers: {}",
            first_headers.join(", ")
        ));
    }
    if result.entries.is_empty() && result.problems.is_empty() {
        return Err("no connections found".into());
    }
    Ok(result)
}
pub fn parse_table_text(text: &str) -> Result<TableImport, String> {
    let (_, rows) = detect_delimiter(text)?;
    parse_rows(rows)
}
pub fn read_table(path: impl AsRef<Path>) -> Result<TableImport, String> {
    let path = path.as_ref();
    let meta = fs::metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if meta.is_dir() {
        return Err(format!("{}: is a folder", path.display()));
    }
    if meta.len() > MAX_FILE_SIZE {
        return Err(format!("{}: file too large (10 MiB limit)", path.display()));
    }
    let bytes = fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if bytes.starts_with(b"PK\x03\x04") {
        let mut book = open_workbook_auto_from_rs(Cursor::new(bytes))
            .map_err(|e| format!("{}: invalid workbook: {e}", path.display()))?;
        let range = book
            .worksheet_range_at(0)
            .ok_or_else(|| format!("{}: workbook has no worksheets", path.display()))?
            .map_err(|e| format!("{}: {e}", path.display()))?;
        let rows = range
            .rows()
            .enumerate()
            .map(|(i, row)| (i + 1, row.iter().map(ToString::to_string).collect()))
            .collect();
        return parse_rows(rows);
    }
    if bytes.starts_with(&[0xd0, 0xcf, 0x11, 0xe0, 0xa1, 0xb1, 0x1a, 0xe1]) {
        return Err(format!(
            "{}: legacy .xls or protected workbook is unsupported; save as .xlsx or CSV",
            path.display()
        ));
    }
    let text = decode_text(&bytes);
    if text.trim_start().starts_with('<') {
        return Err(format!(
            "{}: markup or XML spreadsheet is unsupported; save as .xlsx or CSV",
            path.display()
        ));
    }
    parse_table_text(&text)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_csv_with_title_host_port_user_key_folder_notes() {
        let p=parse_table_text("Server export\nName,Host,Port,User,Password,Key,Folder,Notes\nweb,deploy@web.example:2222,,root,,/keys/web,Production/Web,front end\n").unwrap();
        assert_eq!(p.entries.len(), 1);
        let e = &p.entries[0];
        assert_eq!(e.host.addr, "web.example");
        assert_eq!(e.host.user, "root");
        assert_eq!(e.host.port, 2222);
        assert_eq!(e.key_file, "/keys/web");
        assert_eq!(e.folders, ["Production", "Web"]);
        assert_eq!(e.host.notes, "front end");
    }
    #[test]
    fn parses_chinese_semicolon_ipv6_and_preserves_password_spaces() {
        let p = parse_table_text(
            "名称;主机地址;端口;用户名;密码\nrouter;[2001:db8::1]:2200; ;admin; pass word \n",
        )
        .unwrap();
        assert_eq!(p.entries[0].host.addr, "2001:db8::1");
        assert_eq!(p.entries[0].host.port, 2200);
        assert_eq!(p.entries[0].password.as_deref(), Some(" pass word "));
    }
    #[test]
    fn parses_excel_sep_directive_tabs_and_row_problems() {
        let p = parse_table_text(
            "sep=;\nName;Host;Port\nok;host.example;22\nbad;host.example;70000\nmissing;;22\n",
        )
        .unwrap();
        assert_eq!(p.entries.len(), 1);
        assert_eq!(p.problems.len(), 2);
        assert_eq!(p.problems[0].reason, "bad-port");
        let t = parse_table_text("Host\tUser\nexample.org\troot\n").unwrap();
        assert_eq!(t.entries[0].host.user, "root");
    }
}
