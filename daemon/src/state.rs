use crate::{
    config::{Config, Group, validate_ips},
    scheduler::Schedule,
    speedtest::Job,
};
use anyhow::{Context, Result, ensure};
use portable_atomic::AtomicU64;
use serde::Serialize;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, VecDeque},
    net::IpAddr,
    os::unix::fs::PermissionsExt,
    path::Path,
    sync::{Arc, Mutex, RwLock, atomic::Ordering},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tokio::sync::{Notify, mpsc, oneshot};

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
#[derive(Clone, Serialize)]
pub struct Entry {
    pub ts: u64,
    pub client_ip: IpAddr,
    pub host: String,
    pub path: String,
    pub target: String,
    pub status: u16,
}
#[derive(Default)]
pub struct Counters {
    pub dns: AtomicU64,
    pub srv: AtomicU64,
    pub aaaa: AtomicU64,
    pub http: AtomicU64,
    pub last_redirect: AtomicU64,
}
pub type Shared = Arc<State>;
pub struct State {
    pub config: RwLock<Arc<Config>>,
    pub stats: Counters,
    pub entries: Mutex<VecDeque<Entry>>,
    pub started: Instant,
    pub job: Mutex<Job>,
    pub schedule: Mutex<Schedule>,
    pub tested: Mutex<BTreeMap<String, (f64, u64)>>,
    pub changed: Notify,
    pub mutation: tokio::sync::Mutex<()>,
    pub syncing: tokio::sync::Mutex<()>,
    pub reload: mpsc::Sender<oneshot::Sender<Result<()>>>,
}
impl State {
    pub fn new(c: Config, reload: mpsc::Sender<oneshot::Sender<Result<()>>>) -> Shared {
        Arc::new(Self {
            config: RwLock::new(Arc::new(c)),
            stats: Default::default(),
            entries: Default::default(),
            started: Instant::now(),
            job: Mutex::new(Job::idle()),
            schedule: Mutex::new(Schedule {
                last_run_status: "idle".into(),
                ..Default::default()
            }),
            tested: Default::default(),
            changed: Notify::new(),
            mutation: Default::default(),
            syncing: Default::default(),
            reload,
        })
    }
    pub fn snapshot(&self) -> Arc<Config> {
        self.config.read().unwrap().clone()
    }
    pub fn log(&self, entry: Entry) {
        let capacity = self.snapshot().runtime.log_capacity;
        let mut entries = self.entries.lock().unwrap();
        if capacity > 0 {
            while entries.len() >= capacity {
                entries.pop_front();
            }
            entries.push_back(entry);
        }
    }
    pub fn status(&self) -> Value {
        let c = self.snapshot();
        let s = &self.stats;
        let tested = self.tested.lock().unwrap();
        let mut groups = serde_json::Map::new();
        for (id, g) in &c.groups {
            let (speed, ts) = tested.get(id).copied().unwrap_or_default();
            groups.insert(
                id.clone(),
                json!({
                    "enabled": g.enabled,
                    "ip_family": g.ip_family,
                    "selected_ipv4": g.selected_ipv4,
                    "selected_ipv6": g.selected_ipv6,
                    "last_speed_mbps": speed,
                    "last_tested_ts": ts,
                }),
            );
        }
        let mut schedule = json!(*self.schedule.lock().unwrap());
        schedule["auto_enabled"] = c.speedtest.auto_enabled.into();
        json!({
            "running": true,
            "version": env!("CARGO_PKG_VERSION"),
            "uptime_secs": self.started.elapsed().as_secs(),
            "dns_listen": c.dns.addr(),
            "http_listen": c.http.addr(),
            "redirect_ip": c.http.redirect_ip,
            "dns_queries_total": s.dns.load(Ordering::Relaxed),
            "srv_blocked_total": s.srv.load(Ordering::Relaxed),
            "aaaa_filtered_total": s.aaaa.load(Ordering::Relaxed),
            "http_redirects_total": s.http.load(Ordering::Relaxed),
            "last_redirect_ts": s.last_redirect.load(Ordering::Relaxed),
            "groups": groups,
            "scheduler": schedule,
        })
    }
    pub async fn apply(
        &self,
        id: &str,
        v4: &str,
        v6: &str,
        expected: Option<&Group>,
        scheduled: bool,
    ) -> Result<bool> {
        validate_ips(v4, v6)?;
        let _guard = self.mutation.lock().await;
        let (hook, prev_v4, prev_v6) = {
            let mut config = self.config.write().unwrap();
            let c = Arc::make_mut(&mut config);
            if scheduled
                && (!c.speedtest.auto_enabled
                    || crate::scheduler::active(
                        now(),
                        self.stats.last_redirect.load(Ordering::Relaxed),
                        c.speedtest.defer_active_minutes,
                    ))
            {
                return Ok(false);
            }
            let g = c.groups.get_mut(id).context("unknown group")?;
            ensure!(
                expected.is_none_or(|e| g == e),
                "group changed during speedtest"
            );
            let prev_v4 = std::mem::replace(&mut g.selected_ipv4, v4.into());
            let prev_v6 = std::mem::replace(&mut g.selected_ipv6, v6.into());
            (c.runtime.apply_hook.clone(), prev_v4, prev_v6)
        };
        if let Err(error) = run_apply_hook(&hook, id, v4, v6).await {
            let mut config = self.config.write().unwrap();
            // The mutation lock also serializes reloads, so this group is still present.
            let g = Arc::make_mut(&mut config).groups.get_mut(id).unwrap();
            g.selected_ipv4 = prev_v4;
            g.selected_ipv6 = prev_v6;
            return Err(error);
        }
        Ok(true)
    }
}

async fn run_apply_hook(hook: &Path, id: &str, v4: &str, v6: &str) -> Result<()> {
    if hook.as_os_str().is_empty() {
        return Ok(());
    }
    match tokio::fs::metadata(hook).await {
        Ok(m) if m.is_file() && m.permissions().mode() & 0o111 != 0 => (),
        Ok(_) => return Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e.into()),
    }
    let status = tokio::time::timeout(
        Duration::from_secs(20),
        tokio::process::Command::new(hook)
            .args([id, v4, v6])
            .kill_on_drop(true)
            .status(),
    )
    .await
    .context("apply hook timed out")??;
    ensure!(status.success(), "apply hook exited with {status}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test(start_paused = true)]
    async fn hook_timeout_restores_previous_selection() {
        let dir = tempfile::tempdir().unwrap();
        let hook = dir.path().join("apply-ip");
        std::fs::write(&hook, "#!/bin/sh\nexec sleep 60\n").unwrap();
        std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o700)).unwrap();
        let mut config = Config::default();
        config.runtime.apply_hook = hook;
        let group = config.groups.get_mut("xbox_cn1").unwrap();
        group.selected_ipv4 = "192.0.2.1".into();
        group.selected_ipv6 = "2001:db8::1".into();
        let state = State::new(config, mpsc::channel(1).0);
        let error = state
            .apply("xbox_cn1", "192.0.2.2", "", None, false)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("timed out"));
        let config = state.snapshot();
        let group = &config.groups["xbox_cn1"];
        assert_eq!(group.selected_ipv4, "192.0.2.1");
        assert_eq!(group.selected_ipv6, "2001:db8::1");
    }
}
