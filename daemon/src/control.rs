use crate::{
    config::Family,
    ipdb, net, rules, selfcheck,
    speedtest::{self, Options},
    state::Shared,
};
use anyhow::{Context, Result, bail, ensure};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    io,
    os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio::{
    io::{AsyncWriteExt, BufReader},
    net::{UnixListener, UnixStream},
    sync::{Semaphore, oneshot},
    time::timeout,
};
const LIMIT: usize = 8 * 1024 * 1024;
#[derive(Deserialize)]
struct Request {
    method: String,
    #[serde(default = "object")]
    params: Value,
}
fn object() -> Value {
    json!({})
}
fn string<'a>(p: &'a Value, key: &str, default: &'a str) -> Result<&'a str> {
    match p.get(key) {
        Some(v) => v
            .as_str()
            .with_context(|| format!("{key} must be a string")),
        None => Ok(default),
    }
}
fn required<'a>(p: &'a Value, key: &str) -> Result<&'a str> {
    p.get(key)
        .and_then(Value::as_str)
        .with_context(|| format!("{key} must be a string"))
}
async fn dispatch(s: &Shared, request: Request) -> Result<Value> {
    let p = &request.params;
    ensure!(p.is_object(), "params must be an object");
    let c = s.snapshot();
    match request.method.as_str() {
        "status" => Ok(s.status()),
        "logs" => {
            let n = match p.get("limit") {
                Some(v) => v.as_u64().context("invalid limit")?,
                None => 50,
            };
            Ok(
                json!({"entries":s.entries.lock().unwrap().iter().rev().take(n as usize).collect::<Vec<_>>()}),
            )
        }
        "list_ips" | "speedtest_start" => {
            let id = required(p, "group")?;
            rules::group(id)?;
            let g = &c.groups[id];
            let family: Family = match p.get("ip_family") {
                Some(v) => serde_json::from_value(v.clone())?,
                None => g.ip_family,
            };
            let keyword = string(p, "filter_keyword", &g.filter_keyword)?;
            if request.method == "list_ips" {
                let list = ipdb::list(&c, id, family, keyword).await?;
                Ok(json!({"total":list.ips.len(),"ips":list.ips,"custom_text":list.custom_text}))
            } else {
                let apply = match p.get("auto_apply") {
                    Some(v) => v.as_bool().context("invalid auto_apply")?,
                    None => false,
                };
                speedtest::start(
                    s.clone(),
                    Options {
                        group: id.into(),
                        family,
                        keyword: keyword.into(),
                        apply,
                        scheduled: false,
                    },
                )
            }
        }
        "speedtest_status" => Ok(json!(*s.job.lock().unwrap())),
        "speedtest_cancel" => {
            let j = s.job.lock().unwrap();
            Ok(json!({"cancelled":j.cancel()}))
        }
        "apply_ip" => {
            let id = required(p, "group")?;
            rules::group(id)?;
            let v4 = required(p, "ipv4")?;
            let v6 = required(p, "ipv6")?;
            s.apply(id, v4, v6, None, false).await?;
            Ok(json!({"group":id,"ipv4":v4,"ipv6":v6}))
        }
        "sync_ips" => {
            ipdb::sync(
                s,
                string(p, "mirror", &c.speedtest.ip_sync_mirror)?,
                string(p, "custom_base", &c.speedtest.ip_sync_custom_base)?,
            )
            .await
        }
        "save_custom_ips" => {
            let count = ipdb::save(&c, required(p, "group")?, required(p, "text")?).await?;
            Ok(json!({"count":count}))
        }
        "self_check" => Ok(selfcheck::run(&c).await),
        "reload" => {
            let (tx, rx) = oneshot::channel();
            s.reload.send(tx).await.context("daemon shutting down")?;
            rx.await??;
            Ok(json!({"reloaded":true}))
        }
        _ => bail!("unknown method: {}", request.method),
    }
}
pub fn error(e: impl std::fmt::Display) -> Value {
    json!({"ok":false,"error":e.to_string()})
}
async fn connection(s: Shared, stream: UnixStream) -> Result<()> {
    let mut stream = BufReader::new(stream);
    let result = async {
        let bytes = timeout(
            Duration::from_secs(10),
            net::frame(&mut stream, b"\n", LIMIT),
        )
        .await??;
        dispatch(&s, serde_json::from_slice(&bytes)?).await
    }
    .await;
    let response = match result {
        Ok(v) => json!({"ok":true,"data":v}),
        Err(e) => error(e),
    };
    let mut line = serde_json::to_vec(&response)?;
    line.push(b'\n');
    timeout(Duration::from_secs(10), stream.get_mut().write_all(&line)).await??;
    Ok(())
}
pub struct Socket {
    listener: UnixListener,
    path: PathBuf,
    dev: u64,
    ino: u64,
}
impl Socket {
    pub async fn bind(path: &Path) -> Result<Self> {
        if let Some(dir) = path.parent() {
            tokio::fs::create_dir_all(dir).await?;
        }
        if let Ok(meta) = std::fs::symlink_metadata(path) {
            ensure!(
                meta.file_type().is_socket(),
                "control path already exists and is not a socket"
            );
            match UnixStream::connect(path).await {
                Err(e) if e.kind() == io::ErrorKind::ConnectionRefused => {
                    std::fs::remove_file(path)?
                }
                Ok(_) => bail!("control socket already in use"),
                Err(e) => return Err(e.into()),
            }
        }
        let listener = UnixListener::bind(path)?;
        let m = std::fs::symlink_metadata(path)?;
        let socket = Self {
            listener,
            path: path.into(),
            dev: m.dev(),
            ino: m.ino(),
        };
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
        Ok(socket)
    }
    pub async fn serve(self, s: Shared) {
        let slots = Arc::new(Semaphore::new(16));
        loop {
            let permit = slots.clone().acquire_owned().await.unwrap();
            match self.listener.accept().await {
                Ok((stream, _)) => {
                    let s = s.clone();
                    tokio::spawn(async move {
                        let _permit = permit;
                        let _ = connection(s, stream).await;
                    });
                }
                Err(_) => tokio::time::sleep(Duration::from_millis(100)).await,
            }
        }
    }
}
impl Drop for Socket {
    fn drop(&mut self) {
        if std::fs::symlink_metadata(&self.path)
            .is_ok_and(|m| m.dev() == self.dev && m.ino() == self.ino)
        {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}
pub async fn ctl(path: &Path, method: &str, params: &str) -> Result<Value> {
    let p: Value = serde_json::from_str(params)?;
    let mut line = serde_json::to_vec(&json!({"method":method,"params":p}))?;
    line.push(b'\n');
    timeout(Duration::from_secs(120), async {
        let mut s = BufReader::new(UnixStream::connect(path).await?);
        s.get_mut().write_all(&line).await?;
        let bytes = net::frame(&mut s, b"\n", LIMIT * 4).await?;
        Ok(serde_json::from_slice(&bytes)?)
    })
    .await?
}
