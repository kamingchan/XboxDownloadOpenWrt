use crate::{
    config::{Family, Speed},
    ipdb::{self, Candidate},
    rules, scheduler,
    state::{Shared, now},
};
use anyhow::{Context, Result, bail, ensure};
use futures_util::{StreamExt, stream};
use portable_atomic::AtomicU64;
use reqwest::{
    Client, Url,
    header::{HOST, RANGE},
};
use serde::Serialize;
use serde_json::{Value, json};
use std::{
    collections::HashSet,
    net::{IpAddr, SocketAddr},
    sync::atomic::Ordering,
    time::{Duration, Instant},
};
use tokio::{net::TcpStream, time::timeout};
use tokio_util::sync::CancellationToken;

#[derive(Clone, Serialize)]
pub struct Measurement {
    #[serde(flatten)]
    pub node: Candidate,
    pub rtt_ms: f64,
    pub speed_mbps: f64,
    pub http_status: Option<u16>,
    pub ok: bool,
}
#[derive(Default, Serialize)]
pub struct Job {
    pub running: bool,
    pub job_id: String,
    group: String,
    ip_family: Family,
    stage: &'static str,
    total_candidates: usize,
    tested_count: usize,
    total_to_download: usize,
    error: Option<String>,
    note: Option<String>,
    results: Vec<Measurement>,
    #[serde(skip)]
    token: CancellationToken,
}
impl Job {
    pub fn idle() -> Self {
        Self {
            stage: "idle",
            ..Self::default()
        }
    }
    pub fn cancel(&self) -> bool {
        if !self.running || self.stage == "applying" {
            return false;
        }
        self.token.cancel();
        true
    }
}
#[derive(Clone)]
pub struct Options {
    pub group: String,
    pub family: Family,
    pub keyword: String,
    pub apply: bool,
    pub scheduled: bool,
}
static IDS: AtomicU64 = AtomicU64::new(1);
pub fn begin(s: &Shared, opt: &Options) -> Result<String> {
    rules::group(&opt.group)?;
    let mut j = s.job.lock().unwrap();
    ensure!(!j.running, "a speedtest is already running");
    let id = format!("{}-{}", now(), IDS.fetch_add(1, Ordering::Relaxed));
    *j = Job {
        job_id: id.clone(),
        group: opt.group.clone(),
        ip_family: opt.family,
        running: true,
        stage: "loading",
        ..Job::default()
    };
    Ok(id)
}
pub fn start(s: Shared, opt: Options) -> Result<Value> {
    let id = begin(&s, &opt)?;
    tokio::spawn(async move {
        let _ = run(&s, &opt).await;
    });
    Ok(json!({"job_id":id}))
}
pub async fn run(s: &Shared, opt: &Options) -> Result<String> {
    let token = s.job.lock().unwrap().token.clone();
    let result = tokio::select! {
        biased;
        _ = token.cancelled() => Err(anyhow::anyhow!("cancelled")),
        result = measure(s, opt) => result,
    };
    let mut j = s.job.lock().unwrap();
    j.running = false;
    j.stage = if token.is_cancelled() {
        "cancelled"
    } else if result.is_err() {
        "failed"
    } else {
        "completed"
    };
    j.error = result.as_ref().err().map(ToString::to_string);
    result
}
fn valid_url(text: &str) -> Result<Url> {
    let u = Url::parse(text)?;
    ensure!(
        matches!(u.scheme(), "http" | "https")
            && u.has_host()
            && u.username().is_empty()
            && u.password().is_none(),
        "invalid speedtest URL"
    );
    Ok(u)
}
fn package_url(value: &Value) -> Option<Url> {
    match value {
        Value::String(s) => valid_url(s.trim()).ok(),
        Value::Array(a) => a.iter().find_map(package_url),
        Value::Object(o) => o
            .iter()
            .filter(|(k, _)| {
                ["url", "downloadurl", "uri", "packageuri", "data"]
                    .iter()
                    .any(|name| k.eq_ignore_ascii_case(name))
            })
            .find_map(|(_, v)| package_url(v))
            .or_else(|| {
                o.values()
                    .filter(|v| v.is_object() || v.is_array())
                    .find_map(package_url)
            }),
        _ => None,
    }
}
const PACKAGE_RESOLVER: &str = "https://xbox.skydevil.xyz/Game/GetAppPackage?WuCategoryId=986a47b3-0085-4c0c-b3b3-3b806f969b00";

async fn package(resolver: &str) -> Result<Url> {
    let client = Client::builder()
        .timeout(Duration::from_secs(6))
        .no_proxy()
        .build()?;
    let body = ipdb::bounded_get(&client, resolver, 1024 * 1024).await?;
    valid_url(std::str::from_utf8(&body)?.trim())
        .ok()
        .or_else(|| {
            serde_json::from_slice(&body)
                .ok()
                .as_ref()
                .and_then(package_url)
        })
        .context("package resolver returned no download URL")
}

struct Targets {
    urls: Vec<Url>,
    note: Option<String>,
}
impl Targets {
    fn port(&self) -> u16 {
        self.urls
            .first()
            .and_then(Url::port_or_known_default)
            .unwrap_or(80)
    }
    async fn load(id: &str, custom: &str, resolver: &str) -> Result<Self> {
        let rule = rules::group(id)?;
        let urls = if !custom.is_empty() {
            vec![valid_url(custom)?]
        } else if !rule.urls.is_empty() {
            rule.urls
                .iter()
                .map(|u| valid_url(u))
                .collect::<Result<_>>()?
        } else {
            match package(resolver).await {
                Ok(url) => vec![url],
                Err(e) => {
                    return Ok(Self {
                        urls: vec![],
                        note: Some(format!(
                            "Package resolver unavailable ({e}). Ranking by TCP RTT only."
                        )),
                    });
                }
            }
        };
        Ok(Self { urls, note: None })
    }
}
async fn rtt(node: Candidate, port: u16, timeout_ms: u32) -> Option<Measurement> {
    let start = Instant::now();
    let addr = SocketAddr::new(node.ip, port);
    timeout(
        Duration::from_millis(timeout_ms.into()),
        TcpStream::connect(addr),
    )
    .await
    .ok()?
    .ok()?;
    Some(Measurement {
        node,
        rtt_ms: start.elapsed().as_secs_f64() * 1000.,
        speed_mbps: 0.,
        http_status: None,
        ok: true,
    })
}
async fn download(node: &mut Measurement, url: &Url, settings: &Speed) -> Result<()> {
    let mut endpoint = url.clone();
    let host = url.host_str().context("missing host")?;
    let authority = &url[url::Position::BeforeHost..url::Position::AfterPort];
    let addr = SocketAddr::new(
        node.node.ip,
        url.port_or_known_default().context("missing port")?,
    );
    // An HTTP literal always dials the candidate, even when the configured URL also uses a literal.
    let builder = Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .http1_only()
        .timeout(Duration::from_secs(settings.download_timeout_secs.into()));
    let builder = if url.scheme() == "http" {
        endpoint
            .set_ip_host(node.node.ip)
            .map_err(|_| anyhow::anyhow!("invalid candidate address"))?;
        builder
    } else {
        if let Ok(literal) = host.trim_matches(['[', ']']).parse::<IpAddr>() {
            ensure!(
                literal == node.node.ip,
                "HTTPS literal differs from candidate"
            );
        }
        builder.resolve(host, addr)
    };
    let bytes_wanted = u64::from(settings.download_range_mb) * 1024 * 1024;
    let client = builder.build()?;
    let start = Instant::now();
    let mut res = client
        .get(endpoint)
        .header(HOST, authority)
        .header("User-Agent", "XboxDownload")
        .header(RANGE, format!("bytes=0-{}", bytes_wanted - 1))
        .send()
        .await?;
    let status = res.status().as_u16();
    node.http_status = Some(status);
    ensure!(matches!(status, 200 | 206), "HTTP {status}");
    let mut read = 0u64;
    while read < bytes_wanted {
        match res.chunk().await {
            Ok(Some(chunk)) => read += (chunk.len() as u64).min(bytes_wanted - read),
            Ok(None) => break,
            Err(e) if read > 0 && e.is_timeout() => break,
            Err(e) => return Err(e.into()),
        }
    }
    ensure!(read > 0, "empty HTTP body");
    node.speed_mbps = read as f64 / 1_000_000. / start.elapsed().as_secs_f64().max(1e-9);
    Ok(())
}
async fn probe(mut m: Measurement, urls: &[Url], settings: &Speed) -> Measurement {
    for url in urls {
        if download(&mut m, url, settings).await.is_ok() {
            return m;
        }
    }
    m.ok = false;
    m
}
fn shortlist(mut candidates: Vec<Measurement>, per_family: usize) -> Vec<Measurement> {
    candidates.sort_by(|a, b| a.rtt_ms.total_cmp(&b.rtt_ms));
    let (mut v4, mut v6) = (0, 0);
    candidates.retain(|m| {
        let count = if m.node.ip.is_ipv4() {
            &mut v4
        } else {
            &mut v6
        };
        *count += 1;
        *count <= per_family
    });
    candidates
}

async fn measure(s: &Shared, opt: &Options) -> Result<String> {
    let c = s.snapshot();
    let g = &c.groups[&opt.group];
    let settings = &c.speedtest;
    let targets = Targets::load(&opt.group, &g.custom_test_url, PACKAGE_RESOLVER).await?;
    let port = targets.port();
    s.job.lock().unwrap().note = targets.note;
    let urls = targets.urls;
    let mut candidates = ipdb::list(&c, &opt.group, opt.family, &opt.keyword)
        .await?
        .ips;
    let mut incumbents = Vec::new();
    let mut baseline_only = Vec::new();
    if opt.apply && !urls.is_empty() {
        for ip in [&g.selected_ipv4, &g.selected_ipv6]
            .into_iter()
            .filter_map(|value| value.parse().ok())
            .filter(|ip| opt.family.allows(*ip))
        {
            incumbents.push(ip);
            if !candidates.iter().any(|node| node.ip == ip) {
                baseline_only.push(ip);
                candidates.push(Candidate::new(ip, String::new()));
            }
        }
    }
    {
        let mut j = s.job.lock().unwrap();
        j.stage = "prefiltering";
        j.total_candidates = candidates.len();
    }
    let mut fast = Vec::new();
    let mut tasks = stream::iter(candidates)
        .map(|n| rtt(n, port, settings.tcp_timeout_ms))
        .buffer_unordered(settings.prefilter_concurrency);
    while let Some(m) = tasks.next().await {
        if let Some(m) = m {
            fast.push(m);
        }
        s.job.lock().unwrap().tested_count += 1;
    }
    let mut baseline = Vec::new();
    let mut batch: Vec<_> = fast
        .iter()
        .filter(|m| incumbents.contains(&m.node.ip))
        .cloned()
        .collect();
    fast.retain(|m| !baseline_only.contains(&m.node.ip));
    let mut results = shortlist(fast, settings.prefilter_top_n);
    if !urls.is_empty() {
        // Measure each incumbent once in the same download pool as the challengers.
        // A filtered-out incumbent is still a baseline, but cannot win as a new candidate.
        let finalists: HashSet<_> = results.iter().map(|m| m.node.ip).collect();
        batch.extend(
            results
                .into_iter()
                .filter(|m| !incumbents.contains(&m.node.ip)),
        );
        {
            let mut j = s.job.lock().unwrap();
            j.stage = "downloading";
            j.tested_count = 0;
            j.total_to_download = batch.len();
        }
        let mut downloaded = Vec::new();
        let mut tasks = stream::iter(batch)
            .map(|m| probe(m, &urls, settings))
            .buffer_unordered(settings.download_concurrency);
        while let Some(m) = tasks.next().await {
            let mut j = s.job.lock().unwrap();
            j.tested_count += 1;
            if incumbents.contains(&m.node.ip) {
                baseline.push(m.clone());
            }
            if finalists.contains(&m.node.ip) {
                j.results.push(m.clone());
                downloaded.push(m);
            }
        }
        downloaded.sort_by(|a, b| {
            b.ok.cmp(&a.ok)
                .then_with(|| b.speed_mbps.total_cmp(&a.speed_mbps))
                .then_with(|| a.rtt_ms.total_cmp(&b.rtt_ms))
        });
        results = downloaded;
    }
    s.job.lock().unwrap().results = results.clone();
    let Some(best) = results.iter().find(|m| m.ok) else {
        bail!("no reachable candidate passed testing for {}", opt.group)
    };
    s.tested
        .lock()
        .unwrap()
        .insert(opt.group.clone(), (best.speed_mbps, now()));
    if !opt.apply || urls.is_empty() {
        return Ok("tested".into());
    }
    let mut v4 = g.selected_ipv4.clone();
    let mut v6 = g.selected_ipv6.clone();
    for (family, slot) in [(Family::Ipv4, &mut v4), (Family::Ipv6, &mut v6)] {
        let current = baseline
            .iter()
            .find(|m| m.node.family == family && m.ok)
            .map(|m| m.speed_mbps);
        if let Some(winner) = results.iter().find(|m| m.ok && m.node.family == family)
            && scheduler::switch(current, Some(winner.speed_mbps), settings)
        {
            *slot = winner.node.ip.to_string();
        }
    }
    if (v4.as_str(), v6.as_str()) != (g.selected_ipv4.as_str(), g.selected_ipv6.as_str()) {
        // Once the in-memory selection changes, its persistence hook must run to completion.
        s.job.lock().unwrap().stage = "applying";
        let applied = s
            .apply(&opt.group, &v4, &v6, Some(g), opt.scheduled)
            .await?;
        return Ok(if applied { "switched" } else { "deferred" }.into());
    }
    Ok("kept".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn many_fast_ipv4_nodes_cannot_displace_a_reachable_ipv6_node() {
        let measurement = |ip, rtt_ms| Measurement {
            node: Candidate::new(ip, String::new()),
            rtt_ms,
            speed_mbps: 0.,
            http_status: None,
            ok: true,
        };
        let ipv6 = "2001:db8::1".parse().unwrap();
        let mut candidates = vec![measurement(ipv6, 500.)];
        for i in (1..=8).rev() {
            candidates.push(measurement(IpAddr::from([192, 0, 2, i]), f64::from(i)));
        }
        let nodes: Vec<_> = shortlist(candidates, 2).iter().map(|m| m.node.ip).collect();
        assert_eq!(
            nodes,
            [
                IpAddr::from([192, 0, 2, 1]),
                IpAddr::from([192, 0, 2, 2]),
                ipv6
            ]
        );
    }
    #[tokio::test]
    async fn offline_resolver_uses_tcp_port_80_and_custom_url_takes_precedence() {
        use tokio::{
            io::{AsyncReadExt, AsyncWriteExt},
            net::TcpListener,
        };
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let resolver = format!("http://{}/lookup", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (mut connection, _) = listener.accept().await.unwrap();
            let read = connection.read(&mut [0; 1024]).await.unwrap();
            assert!(read > 0);
            connection
                .write_all(b"HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\n\r\n")
                .await
                .unwrap();
        });
        let targets = Targets::load("xbox_app", "", &resolver).await.unwrap();
        server.await.unwrap();
        assert!(targets.urls.is_empty());
        assert_eq!(targets.port(), 80);
        assert!(targets.note.unwrap().contains("TCP RTT only"));
        let targets = Targets::load("xbox_app", "http://example.test:8080/package", &resolver)
            .await
            .unwrap();
        assert_eq!(targets.port(), 8080);
        assert!(targets.note.is_none());
    }
    #[test]
    fn package_responses_support_nested_urls_and_reject_error_envelopes() {
        let url = "https://dl.delivery.mp.microsoft.com/test?sig=abc";
        for data in [
            json!({"data":url}),
            json!({"packages":[{"DownloadUrl":url}]}),
            json!(url),
        ] {
            assert_eq!(package_url(&data).unwrap().as_str(), url);
        }
        assert!(package_url(&json!({"code":"401","msg":"Unauthorized"})).is_none());
    }
}
