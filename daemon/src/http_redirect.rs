use crate::{
    net, rules,
    state::{Entry, Shared, now},
};
use anyhow::{Context, Result, bail, ensure};
use std::{
    net::{IpAddr, SocketAddr},
    sync::atomic::Ordering,
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt, BufReader},
    net::{TcpListener, TcpStream},
    time::timeout,
};
struct Request {
    host: String,
    path: String,
    keep: bool,
}
fn parse(bytes: &[u8]) -> Result<Request> {
    ensure!(
        bytes
            .split_inclusive(|b| *b == b'\n')
            .all(|line| line.ends_with(b"\r\n")),
        "invalid line ending"
    );
    let mut headers = [httparse::EMPTY_HEADER; 128];
    let mut req = httparse::Request::new(&mut headers);
    ensure!(req.parse(bytes)?.is_complete(), "incomplete request");
    let path = req.path.context("missing path")?;
    ensure!(
        path.starts_with('/') && !path.bytes().any(|b| b.is_ascii_control()),
        "invalid request target"
    );
    let mut host = None;
    let mut keep = req.version == Some(1);
    for h in req.headers.iter() {
        let val = std::str::from_utf8(h.value)?.trim();
        if h.name.eq_ignore_ascii_case("Host") {
            ensure!(host.is_none(), "duplicate Host");
            let value = if let Some((name, port)) = val.rsplit_once(':')
                && !name.contains(':')
            {
                port.parse::<u16>()?;
                name
            } else {
                val
            };
            host = Some(value.trim_end_matches('.').to_ascii_lowercase());
        }
        if h.name.eq_ignore_ascii_case("Connection")
            && val
                .split(',')
                .any(|v| v.trim().eq_ignore_ascii_case("close"))
        {
            keep = false;
        }
        if h.name.eq_ignore_ascii_case("Transfer-Encoding")
            || (h.name.eq_ignore_ascii_case("Content-Length") && val != "0")
        {
            bail!("request body unsupported");
        }
    }
    Ok(Request {
        host: host.context("missing Host")?,
        path: path.to_owned(),
        keep,
    })
}
async fn connection(s: Shared, stream: TcpStream, ip: IpAddr) -> Result<()> {
    let mut reader = BufReader::new(stream);
    loop {
        let frame = timeout(
            Duration::from_secs(15),
            net::frame(&mut reader, b"\r\n\r\n", 16384),
        )
        .await?;
        if frame.as_ref().is_ok_and(|f| f.is_empty()) {
            return Ok(());
        }
        let request = frame.and_then(|f| parse(&f));
        let Ok(r) = request else {
            reader
                .get_mut()
                .write_all(
                    b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .await?;
            // Send FIN before discarding any queued bytes of an oversized request.
            // Closing with unread bytes can reset the connection before the client sees 400.
            reader.get_mut().shutdown().await?;
            let _ = timeout(
                Duration::from_millis(200),
                tokio::io::copy(&mut reader.take(32768), &mut tokio::io::sink()),
            )
            .await;
            return Ok(());
        };
        let target = rules::target(&r.host).map(|h| format!("http://{h}{}", r.path));
        let (status, reason, location) = match &target {
            Some(t) => (302, "Found", format!("Location: {t}\r\n")),
            None => (404, "Not Found", String::new()),
        };
        let conn = if r.keep { "keep-alive" } else { "close" };
        let reply = format!(
            "HTTP/1.1 {status} {reason}\r\n{location}Content-Length: 0\r\nConnection: {conn}\r\n\r\n"
        );
        timeout(
            Duration::from_secs(5),
            reader.get_mut().write_all(reply.as_bytes()),
        )
        .await??;
        let ts = now();
        if target.is_some() {
            s.stats.http.fetch_add(1, Ordering::Relaxed);
            s.stats.last_redirect.store(ts, Ordering::Relaxed);
        }
        s.log(Entry {
            ts,
            client_ip: ip,
            host: r.host,
            path: r.path,
            target: target.unwrap_or_default(),
            status,
        });
        if !r.keep {
            return Ok(());
        }
    }
}
pub async fn serve(s: Shared, lis: TcpListener) {
    net::serve_tcp(lis, move |stream, peer: SocketAddr| {
        connection(s.clone(), stream, peer.ip())
    })
    .await
}
