use crate::{
    config::Speed,
    ipdb, rules,
    speedtest::{self, Options},
    state::{Shared, now},
};
use chrono::{DateTime, Local, Timelike};
use serde::Serialize;
use std::{sync::atomic::Ordering, time::Duration};
#[derive(Default, Serialize)]
pub struct Schedule {
    pub next_run_ts: u64,
    pub last_run_ts: u64,
    pub last_run_status: String,
    pub last_run_message: String,
}
pub fn active(now: u64, last: u64, minutes: u32) -> bool {
    last != 0 && now.saturating_sub(last) < u64::from(minutes) * 60
}
pub fn switch(current: Option<f64>, best: Option<f64>, s: &Speed) -> bool {
    best.is_some_and(|best| {
        current.is_none_or(|v| {
            (v < s.min_speed_mbps && best > v) || best >= v * (1. + s.switch_threshold_pct / 100.)
        })
    })
}
fn first(now: u64, s: &Speed) -> u64 {
    if s.prefer_hour < 0 {
        return now + u64::from(s.interval_hours) * 3600;
    }
    // Iterate actual local hours so DST skips/repeats remain valid.
    let local = DateTime::from_timestamp(now as i64, 0)
        .unwrap()
        .with_timezone(&Local);
    let top = now - u64::from(local.minute() * 60 + local.second());
    (1..=49)
        .map(|i| top + i * 3600)
        .find(|ts| {
            DateTime::from_timestamp(*ts as i64, 0)
                .unwrap()
                .with_timezone(&Local)
                .hour()
                == s.prefer_hour as u32
        })
        .unwrap_or(now + 86400)
}
fn record(s: &Shared, status: &str, message: String) {
    let mut sc = s.schedule.lock().unwrap();
    sc.last_run_ts = now();
    sc.last_run_status = status.into();
    sc.last_run_message = message;
}
pub async fn serve(s: Shared) {
    loop {
        let c = s.snapshot();
        if !c.speedtest.auto_enabled {
            s.schedule.lock().unwrap().next_run_ts = 0;
            s.changed.notified().await;
            continue;
        }
        let mut next = first(now(), &c.speedtest);
        loop {
            s.schedule.lock().unwrap().next_run_ts = next;
            tokio::select! {
                _ = s.changed.notified() => break,
                _ = tokio::time::sleep(Duration::from_secs(next.saturating_sub(now()))) => {},
            }
            if active(
                now(),
                s.stats.last_redirect.load(Ordering::Relaxed),
                c.speedtest.defer_active_minutes,
            ) || s.job.lock().unwrap().running
            {
                record(
                    &s,
                    "deferred",
                    "Recent HTTP redirect or another speedtest is active".into(),
                );
                next = now() + 900;
                continue;
            }
            let mut messages = vec![];
            let mut failed = false;
            let mut switched = false;
            let mut deferred = false;
            if c.speedtest.ip_sync_enabled {
                match ipdb::sync(
                    &s,
                    &c.speedtest.ip_sync_mirror,
                    &c.speedtest.ip_sync_custom_base,
                )
                .await
                {
                    Ok(v) if v["all_synced"] == true => (),
                    _ => {
                        failed = true;
                        messages.push("IP mirror sync incomplete, using local lists".into());
                    }
                }
            }
            for rule in rules::GROUPS {
                let cfg = s.snapshot();
                if !cfg.speedtest.auto_enabled {
                    break;
                }
                let g = &cfg.groups[rule.id];
                if !g.enabled {
                    continue;
                }
                if active(
                    now(),
                    s.stats.last_redirect.load(Ordering::Relaxed),
                    cfg.speedtest.defer_active_minutes,
                ) {
                    deferred = true;
                    messages.push("Recent HTTP redirect".into());
                    break;
                }
                let opt = Options {
                    group: rule.id.into(),
                    family: g.ip_family,
                    keyword: g.filter_keyword.clone(),
                    apply: true,
                    scheduled: true,
                };
                if speedtest::begin(&s, &opt).is_err() {
                    deferred = true;
                    messages.push("Another speedtest is active".into());
                    break;
                }
                match speedtest::run(&s, &opt).await {
                    Ok(status) => {
                        if status == "tested" {
                            failed = true;
                            messages
                                .push(format!("{} used TCP RTT only; selection retained", rule.id));
                        }
                        switched |= status == "switched";
                        deferred |= status == "deferred";
                        let latest = s.snapshot();
                        let g = &latest.groups[rule.id];
                        messages.push(format!(
                            "{} {status} {} {}",
                            rule.id, g.selected_ipv4, g.selected_ipv6
                        ));
                    }
                    Err(e) => {
                        failed = true;
                        messages.push(format!("{}: {e}", rule.id));
                    }
                }
                if deferred {
                    break;
                }
            }
            let status = if deferred {
                "deferred"
            } else if failed {
                "warning"
            } else if switched {
                "switched"
            } else {
                "kept"
            };
            record(&s, status, messages.join("; "));
            next = now()
                + if deferred {
                    900
                } else {
                    u64::from(c.speedtest.interval_hours) * 3600
                };
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn downloads_defer_until_quiet() {
        assert!(active(150, 100, 1));
        assert!(!active(160, 100, 1));
        assert!(!active(20, 0, 15));
    }
    #[tokio::test(start_paused = true)]
    async fn scheduled_failures_preserve_the_selected_ip_after_deferral() {
        use crate::{config::Config, state::State};
        let dir = tempfile::tempdir().unwrap();
        let mut config = Config::default();
        config.runtime.data_dir = dir.path().into();
        config.runtime.cache_dir = dir.path().into();
        config.speedtest.ip_sync_enabled = false;
        config.speedtest.prefer_hour = -1;
        config.speedtest.interval_hours = 1;
        for (id, group) in &mut config.groups {
            group.enabled = id == "xbox_cn1";
        }
        let group = config.groups.get_mut("xbox_cn1").unwrap();
        group.selected_ipv4 = "127.0.0.1".into();
        let socket = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        group.custom_test_url = format!(
            "http://localhost:{}/test",
            socket.local_addr().unwrap().port()
        );
        drop(socket);
        let state = State::new(config, tokio::sync::mpsc::channel(1).0);
        state.stats.last_redirect.store(now(), Ordering::Relaxed);
        let task = tokio::spawn(serve(state.clone()));
        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_secs(3601)).await;
        tokio::task::yield_now().await;
        assert_eq!(state.schedule.lock().unwrap().last_run_status, "deferred");
        state.stats.last_redirect.store(0, Ordering::Relaxed);
        tokio::time::advance(Duration::from_secs(901)).await;
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        while state.schedule.lock().unwrap().last_run_status != "warning" {
            assert!(
                std::time::Instant::now() < deadline,
                "scheduler did not report failed probes"
            );
            tokio::task::yield_now().await;
        }
        assert_eq!(
            state.snapshot().groups["xbox_cn1"].selected_ipv4,
            "127.0.0.1"
        );
        task.abort();
    }
    #[test]
    fn first_run_observes_preferred_local_hour_or_interval() {
        let current = now();
        let mut s = Speed {
            prefer_hour: -1,
            interval_hours: 6,
            ..Speed::default()
        };
        assert_eq!(first(current, &s), current + 6 * 3600);
        s.prefer_hour = 4;
        let ts = first(current, &s);
        assert!(ts > current);
        let date = DateTime::from_timestamp(ts as i64, 0)
            .unwrap()
            .with_timezone(&Local);
        assert_eq!((date.hour(), date.minute(), date.second()), (4, 0, 0));
    }
    #[test]
    fn hysteresis_and_failure() {
        let s = Speed::default();
        assert!(!switch(Some(10.), Some(11.99), &s));
        assert!(switch(Some(10.), Some(12.), &s));
        assert!(switch(Some(4.), Some(5.), &s));
        assert!(switch(None, Some(3.), &s));
        assert!(!switch(Some(10.), None, &s));
    }
}
