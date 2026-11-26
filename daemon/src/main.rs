mod config;
mod control;
mod dns;
mod http_redirect;
mod ipdb;
mod net;
mod rules;
mod scheduler;
mod selfcheck;
mod speedtest;
mod state;
use anyhow::{Context, Result, bail, ensure};
use config::Config;
use state::Shared;
use std::{collections::BTreeMap, path::Path, sync::Arc};
use tokio::{
    net::{TcpListener, UdpSocket},
    signal::unix::{SignalKind, signal},
    sync::mpsc,
    task::JoinHandle,
};

#[derive(Default)]
struct Listeners {
    dns: Vec<JoinHandle<()>>,
    http: Option<JoinHandle<()>>,
    control: Option<JoinHandle<()>>,
}
impl Listeners {
    async fn install(&mut self, s: &Shared, c: Config) -> Result<()> {
        let _guard = s.mutation.lock().await;
        let old = s.snapshot();
        let dns = if self.dns.is_empty() || old.dns.addr() != c.dns.addr() {
            Some((
                UdpSocket::bind(c.dns.addr())
                    .await
                    .context("bind UDP DNS")?,
                TcpListener::bind(c.dns.addr())
                    .await
                    .context("bind TCP DNS")?,
            ))
        } else {
            None
        };
        let http = if self.http.is_none() || old.http.addr() != c.http.addr() {
            Some(
                TcpListener::bind(c.http.addr())
                    .await
                    .context("bind HTTP")?,
            )
        } else {
            None
        };
        let control = if self.control.is_none() || old.runtime.socket_path != c.runtime.socket_path
        {
            Some(
                control::Socket::bind(&c.runtime.socket_path)
                    .await
                    .context("bind control socket")?,
            )
        } else {
            None
        };
        // Publish only after every required endpoint has been successfully reserved.
        {
            let mut entries = s.entries.lock().unwrap();
            let remove = entries.len().saturating_sub(c.runtime.log_capacity);
            entries.drain(..remove);
        }
        *s.config.write().unwrap() = Arc::new(c);
        if let Some((udp, tcp)) = dns {
            for h in self.dns.drain(..) {
                h.abort();
            }
            self.dns.push(tokio::spawn(dns::udp(s.clone(), udp)));
            self.dns.push(tokio::spawn(dns::tcp(s.clone(), tcp)));
        }
        if let Some(http) = http {
            if let Some(h) = self.http.take() {
                h.abort();
            }
            self.http = Some(tokio::spawn(http_redirect::serve(s.clone(), http)));
        }
        if let Some(socket) = control {
            if let Some(h) = self.control.take() {
                h.abort();
            }
            self.control = Some(tokio::spawn(socket.serve(s.clone())));
        }
        Ok(())
    }
    async fn stop(self) {
        for h in self.dns.into_iter().chain(self.http).chain(self.control) {
            h.abort();
            let _ = h.await;
        }
    }
}
async fn run(path: &Path) -> Result<()> {
    let mut term = signal(SignalKind::terminate())?;
    let mut int = signal(SignalKind::interrupt())?;
    let mut hup = signal(SignalKind::hangup())?;
    let c = Config::load(path).await?;
    let (tx, mut rx) = mpsc::channel(8);
    let s = state::State::new(c.clone(), tx);
    let mut listeners = Listeners::default();
    listeners.install(&s, c).await?;
    let schedule = tokio::spawn(scheduler::serve(s.clone()));
    loop {
        let channel = tokio::select! {
            _ = term.recv() => break,
            _ = int.recv() => break,
            _ = hup.recv() => None,
            request = rx.recv() => request,
        };
        let result = async {
            listeners.install(&s, Config::load(path).await?).await?;
            s.changed.notify_one();
            Ok(())
        }
        .await;
        if let Some(tx) = channel {
            let _ = tx.send(result);
        } else if let Err(e) = result {
            eprintln!("reload: {e:#}");
        }
    }
    listeners.stop().await;
    s.job.lock().unwrap().cancel();
    // Let an already committed selection finish its persistence hook before shutdown.
    let _guard = s.mutation.lock().await;
    schedule.abort();
    Ok(())
}
fn args(command: &str, iter: impl Iterator<Item = String>) -> Result<BTreeMap<String, String>> {
    let allowed = match command {
        "run" | "print-dnsmasq" => &["--config"][..],
        "ctl" => &["--socket", "--method", "--params"][..],
        _ => bail!(
            "usage: xboxdownloadd run|print-dnsmasq --config PATH | ctl --socket PATH --method NAME [--params JSON]"
        ),
    };
    let mut iter = iter;
    let mut map = BTreeMap::new();
    while let Some(k) = iter.next() {
        ensure!(allowed.contains(&k.as_str()), "unknown option {k}");
        let v = iter
            .next()
            .with_context(|| format!("missing value for {k}"))?;
        ensure!(map.insert(k.clone(), v).is_none(), "duplicate {k}");
    }
    Ok(map)
}
async fn execute(command: &str, iter: impl Iterator<Item = String>) -> Result<bool> {
    let args = args(command, iter)?;
    let required = |k: &str| args.get(k).with_context(|| format!("missing {k}"));
    match command {
        "ctl" => {
            let v = control::ctl(
                Path::new(required("--socket")?),
                required("--method")?,
                args.get("--params").map_or("{}", String::as_str),
            )
            .await?;
            println!("{v}");
            Ok(v["ok"] == true)
        }
        "run" => {
            run(Path::new(required("--config")?)).await?;
            Ok(true)
        }
        "print-dnsmasq" => {
            let c = Config::load(Path::new(required("--config")?)).await?;
            for host in rules::CORE
                .iter()
                .chain(rules::LEGACY)
                .chain(rules::SRV)
                .chain(
                    rules::GROUPS
                        .iter()
                        .filter(|r| c.groups[r.id].enabled)
                        .flat_map(|r| r.hosts),
                )
            {
                println!("server=/{host}/{}#{}", c.dns.bind, c.dns.port);
            }
            Ok(true)
        }
        _ => unreachable!(),
    }
}
#[tokio::main(flavor = "current_thread")]
async fn main() -> std::process::ExitCode {
    let mut argv = std::env::args().skip(1);
    let cmd = argv.next().unwrap_or_default();
    match execute(&cmd, argv).await {
        Ok(true) => std::process::ExitCode::SUCCESS,
        Ok(false) => std::process::ExitCode::FAILURE,
        Err(e) => {
            if cmd == "ctl" {
                println!("{}", control::error(format!("{e:#}")));
            } else {
                eprintln!("{e:#}");
            }
            std::process::ExitCode::FAILURE
        }
    }
}
