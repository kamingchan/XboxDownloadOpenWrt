use crate::rules;
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    path::{Path, PathBuf},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Family {
    #[default]
    Ipv4,
    Ipv6,
    Dual,
}
impl Family {
    pub fn allows(self, ip: IpAddr) -> bool {
        self == Self::Dual || (self == Self::Ipv4) == ip.is_ipv4()
    }
}
#[derive(Clone, Deserialize, Serialize, PartialEq)]
#[serde(default)]
pub struct Group {
    pub enabled: bool,
    pub ip_family: Family,
    pub selected_ipv4: String,
    pub selected_ipv6: String,
    pub filter_keyword: String,
    pub custom_test_url: String,
    pub use_akamai_fallback: bool,
}
impl Default for Group {
    fn default() -> Self {
        Self {
            enabled: true,
            ip_family: Family::Ipv4,
            selected_ipv4: String::new(),
            selected_ipv6: String::new(),
            filter_keyword: String::new(),
            custom_test_url: String::new(),
            use_akamai_fallback: false,
        }
    }
}
impl Group {
    pub fn selected(&self) -> impl Iterator<Item = IpAddr> + '_ {
        [&self.selected_ipv4, &self.selected_ipv6]
            .into_iter()
            .filter_map(|s| s.parse().ok())
            .filter(|ip| self.ip_family.allows(*ip))
    }
}
#[derive(Clone, Deserialize)]
#[serde(default)]
pub struct Dns {
    pub bind: IpAddr,
    pub port: u16,
    pub mode: String,
    pub upstream: SocketAddr,
    pub ttl: u32,
}
impl Default for Dns {
    fn default() -> Self {
        Self {
            bind: Ipv4Addr::LOCALHOST.into(),
            port: 5353,
            mode: "dnsmasq".into(),
            upstream: SocketAddr::from(([223, 5, 5, 5], 53)),
            ttl: 60,
        }
    }
}
impl Dns {
    pub fn addr(&self) -> SocketAddr {
        (self.bind, self.port).into()
    }
}
#[derive(Clone, Deserialize)]
#[serde(default)]
pub struct Http {
    pub mode: String,
    pub bind: IpAddr,
    pub port: u16,
    pub redirect_ip: Ipv4Addr,
}
impl Default for Http {
    fn default() -> Self {
        Self {
            mode: "alias_ip".into(),
            bind: Ipv4Addr::LOCALHOST.into(),
            port: 18080,
            redirect_ip: Ipv4Addr::LOCALHOST,
        }
    }
}
impl Http {
    pub fn addr(&self) -> SocketAddr {
        (self.bind, self.port).into()
    }
}
#[derive(Clone, Deserialize)]
#[serde(default)]
pub struct Runtime {
    pub socket_path: PathBuf,
    pub data_dir: PathBuf,
    pub cache_dir: PathBuf,
    #[serde(default)]
    pub custom_dir: PathBuf,
    pub apply_hook: PathBuf,
    pub log_capacity: usize,
}
impl Runtime {
    pub fn custom_root(&self) -> &Path {
        if self.custom_dir.as_os_str().is_empty() {
            &self.cache_dir
        } else {
            &self.custom_dir
        }
    }
}
impl Default for Runtime {
    fn default() -> Self {
        Self {
            socket_path: "/var/run/xboxdownload.sock".into(),
            data_dir: "/usr/share/xboxdownload/ip".into(),
            cache_dir: "/var/lib/xboxdownload".into(),
            custom_dir: PathBuf::new(),
            apply_hook: "/usr/libexec/xboxdownload/apply-ip".into(),
            log_capacity: 200,
        }
    }
}
#[derive(Clone, Deserialize)]
#[serde(default)]
pub struct Speed {
    pub auto_enabled: bool,
    pub interval_hours: u32,
    pub prefer_hour: i32,
    pub defer_active_minutes: u32,
    pub switch_threshold_pct: f64,
    pub min_speed_mbps: f64,
    pub prefilter_concurrency: usize,
    pub prefilter_top_n: usize,
    pub download_concurrency: usize,
    pub download_range_mb: u32,
    pub download_timeout_secs: u32,
    pub tcp_timeout_ms: u32,
    pub ip_sync_enabled: bool,
    pub ip_sync_mirror: String,
    pub ip_sync_custom_base: String,
}
impl Default for Speed {
    fn default() -> Self {
        Self {
            auto_enabled: true,
            interval_hours: 12,
            prefer_hour: 4,
            defer_active_minutes: 15,
            switch_threshold_pct: 20.,
            min_speed_mbps: 5.,
            prefilter_concurrency: 24,
            prefilter_top_n: 10,
            download_concurrency: 4,
            download_range_mb: 8,
            download_timeout_secs: 6,
            tcp_timeout_ms: 1500,
            ip_sync_enabled: true,
            ip_sync_mirror: "github".into(),
            ip_sync_custom_base: String::new(),
        }
    }
}
#[derive(Clone, Deserialize)]
#[serde(default)]
pub struct Config {
    pub dns: Dns,
    pub http: Http,
    pub runtime: Runtime,
    pub groups: BTreeMap<String, Group>,
    pub speedtest: Speed,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            dns: Dns::default(),
            http: Http::default(),
            runtime: Runtime::default(),
            groups: rules::GROUPS
                .iter()
                .map(|r| {
                    (
                        r.id.to_owned(),
                        Group {
                            enabled: r.id != "akamai",
                            ..Group::default()
                        },
                    )
                })
                .collect(),
            speedtest: Speed::default(),
        }
    }
}
impl Config {
    /// Addresses served by DNS and delegated by the generated dnsmasq fragment.
    pub fn cdn_addresses(&self, id: &str) -> Vec<IpAddr> {
        let group = &self.groups[id];
        if !group.enabled {
            return Vec::new();
        }
        let selected: Vec<_> = group.selected().collect();
        if selected.is_empty() && group.use_akamai_fallback && self.groups["akamai"].enabled {
            return self.groups["akamai"]
                .selected()
                .filter(|ip| group.ip_family.allows(*ip))
                .collect();
        }
        selected
    }

    pub async fn load(path: &Path) -> Result<Self> {
        let bytes = tokio::fs::read(path)
            .await
            .with_context(|| format!("read {}", path.display()))?;
        // Fill individual fields of each named group with its own defaults (Akamai is opt-in).
        let mut value: serde_json::Value = serde_json::from_slice(&bytes)?;
        if let Some(groups) = value.get_mut("groups").and_then(|v| v.as_object_mut()) {
            for rule in rules::GROUPS {
                let g = groups
                    .entry(rule.id)
                    .or_insert_with(|| serde_json::json!({}));
                if let Some(obj) = g.as_object_mut() {
                    obj.entry("enabled").or_insert((rule.id != "akamai").into());
                }
            }
        }
        let c: Self = serde_json::from_value(value)?;
        for (id, g) in &c.groups {
            rules::group(id)?;
            validate_ips(&g.selected_ipv4, &g.selected_ipv6)?;
        }
        ensure!(
            matches!(c.dns.mode.as_str(), "dnsmasq" | "manual")
                && matches!(c.http.mode.as_str(), "alias_ip" | "custom_port"),
            "invalid listener mode"
        );
        ensure!(
            c.dns.port != 0 && c.http.port != 0,
            "configure nonzero listener ports"
        );
        let s = &c.speedtest;
        ensure!(
            s.interval_hours > 0
                && (-1..=23).contains(&s.prefer_hour)
                && s.prefilter_concurrency > 0
                && s.prefilter_top_n > 0
                && s.download_concurrency > 0
                && s.download_range_mb > 0
                && s.download_timeout_secs > 0
                && s.tcp_timeout_ms > 0
                && s.min_speed_mbps >= 0.
                && s.switch_threshold_pct >= 0.,
            "invalid speedtest configuration"
        );
        ensure!(
            !crate::dns::loops(&c.dns),
            "upstream DNS would loop to a local listener"
        );
        Ok(c)
    }
}
pub fn validate_ips(v4: &str, v6: &str) -> Result<()> {
    if !v4.is_empty() {
        v4.parse::<Ipv4Addr>().context("invalid ipv4")?;
    }
    if !v6.is_empty() {
        v6.parse::<Ipv6Addr>().context("invalid ipv6")?;
    }
    Ok(())
}
