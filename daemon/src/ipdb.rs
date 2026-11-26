use crate::{
    config::{Config, Family},
    rules,
    state::Shared,
};
use anyhow::{Context, Result, bail, ensure};
use serde::Serialize;
use serde_json::{Value, json};
use std::{
    collections::HashSet,
    net::IpAddr,
    path::{Path, PathBuf},
    time::Duration,
};
use tokio::io::AsyncWriteExt;
const MAX_SIZE: usize = 4 * 1024 * 1024;
#[derive(Clone, Debug, Serialize)]
pub struct Candidate {
    pub ip: IpAddr,
    pub family: Family,
    pub location: String,
}
impl Candidate {
    pub fn new(ip: IpAddr, location: String) -> Self {
        Self {
            ip,
            family: if ip.is_ipv4() {
                Family::Ipv4
            } else {
                Family::Ipv6
            },
            location,
        }
    }
}
fn entry(line: &str) -> Option<Candidate> {
    let line = line.trim();
    let (ip, description) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
    let location = description.trim();
    let location = location
        .strip_prefix('(')
        .and_then(|s| s.strip_suffix(')'))
        .unwrap_or(location);
    Some(Candidate::new(ip.parse().ok()?, location.into()))
}
async fn optional(path: PathBuf) -> Result<Option<String>> {
    match tokio::fs::read_to_string(&path).await {
        Ok(s) => Ok(Some(s)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e).with_context(|| format!("read {}", path.display())),
    }
}
pub struct IpList {
    pub ips: Vec<Candidate>,
    pub custom_text: String,
}
pub async fn list(c: &Config, id: &str, family: Family, keyword: &str) -> Result<IpList> {
    let rule = rules::group(id)?;
    let mut contents = String::new();
    'source: for dir in [&c.runtime.cache_dir, &c.runtime.data_dir] {
        for file in rule.files {
            if let Some(data) = optional(dir.join(file)).await? {
                contents = data;
                break 'source;
            }
        }
    }
    let custom_text = optional(c.runtime.cache_dir.join(format!("custom.{id}.txt")))
        .await?
        .unwrap_or_default();
    let mut seen = HashSet::new();
    let ips = contents
        .lines()
        .chain(custom_text.lines())
        .filter_map(entry)
        .filter(|e| family.allows(e.ip) && e.location.contains(keyword) && seen.insert(e.ip))
        .collect();
    Ok(IpList { ips, custom_text })
}
async fn replace(path: &Path, body: &[u8]) -> Result<()> {
    let dir = path.parent().context("invalid cache directory")?;
    tokio::fs::create_dir_all(dir).await?;
    let (file, tmp) = tempfile::NamedTempFile::new_in(dir)?.into_parts();
    let mut file = tokio::fs::File::from_std(file);
    file.write_all(body).await?;
    file.sync_all().await?;
    tmp.persist(path)?;
    Ok(())
}
pub async fn save(c: &Config, id: &str, text: &str) -> Result<usize> {
    rules::group(id)?;
    ensure!(text.len() <= MAX_SIZE, "custom list too large");
    let mut count = 0;
    for (index, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        ensure!(entry(line).is_some(), "invalid IP on line {}", index + 1);
        count += 1;
    }
    replace(
        &c.runtime.cache_dir.join(format!("custom.{id}.txt")),
        text.as_bytes(),
    )
    .await?;
    Ok(count)
}
pub async fn bounded_get(client: &reqwest::Client, url: &str, max: usize) -> Result<Vec<u8>> {
    let mut reply = client.get(url).send().await?.error_for_status()?;
    ensure!(
        reply.content_length().is_none_or(|n| n <= max as u64),
        "response too large"
    );
    let mut data = Vec::new();
    while let Some(chunk) = reply.chunk().await? {
        ensure!(chunk.len() <= max - data.len(), "response too large");
        data.extend_from_slice(&chunk);
    }
    Ok(data)
}
async fn sync_file(client: &reqwest::Client, base: &str, name: &str, dir: &Path) -> Result<usize> {
    let body = bounded_get(
        client,
        &format!("{}/{name}", base.trim_end_matches('/')),
        MAX_SIZE,
    )
    .await?;
    let text = std::str::from_utf8(&body)?.trim_start_matches('\u{feff}');
    let mut lines = text.lines();
    let expected = &name[3..name.len() - 4];
    ensure!(
        lines.next().map(str::trim) == Some(expected),
        "expected {expected} header"
    );
    let mut count = 0;
    for line in lines.filter(|l| !l.trim().is_empty()) {
        ensure!(entry(line).is_some(), "invalid IP record in {name}");
        count += 1;
    }
    ensure!(count > 0, "empty IP database");
    replace(&dir.join(name), &body).await?;
    Ok(count)
}
pub async fn sync(s: &Shared, mirror: &str, custom: &str) -> Result<Value> {
    let _lock = s.syncing.try_lock().context("IP sync already running")?;
    let c = s.snapshot();
    let base = match mirror {
        "github" => "https://raw.githubusercontent.com/skydevil88/XboxDownload/master/IP",
        "jsdelivr" => "https://cdn.jsdelivr.net/gh/skydevil88/XboxDownload@master/IP",
        "custom" => custom,
        _ => bail!("unknown IP mirror"),
    };
    let url = reqwest::Url::parse(base)?;
    ensure!(
        matches!(url.scheme(), "http" | "https")
            && url.has_host()
            && url.query().is_none()
            && url.fragment().is_none(),
        "invalid mirror base URL"
    );
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(15))
        .build()?;
    let mut files = serde_json::Map::new();
    let mut all = true;
    for &name in rules::FILES {
        let row = match sync_file(&client, base, name, &c.runtime.cache_dir).await {
            Ok(count) => json!({"ok":true,"count":count}),
            Err(e) => {
                all = false;
                json!({"ok":false,"error":e.to_string(),"count":0})
            }
        };
        files.insert(name.into(), row);
    }
    Ok(json!({"files":files,"all_synced":all}))
}
