use crate::{config::Config, net};
use anyhow::{Result, ensure};
use hickory_proto::{
    op::{Message, MessageType, Query, ResponseCode},
    rr::{Name, RData, RecordType},
};
use serde_json::{Value, json};
use std::{net::SocketAddr, time::Duration};
use tokio::{
    io::{AsyncWriteExt, BufReader},
    net::{TcpStream, UdpSocket},
    time::timeout,
};
async fn dns(c: &Config, name: &str, ty: RecordType) -> Result<()> {
    let dest = net::reachable(c.dns.addr());
    let s = UdpSocket::bind(if dest.is_ipv4() {
        "0.0.0.0:0"
    } else {
        "[::]:0"
    })
    .await?;
    s.connect(dest).await?;
    let mut q = Message::query();
    q.queries.push(Query::query(
        Name::from_ascii(format!("{}.", name.trim_end_matches('.')))?,
        ty,
    ));
    s.send(&q.to_vec()?).await?;
    let mut buf = [0; 2048];
    let n = s.recv(&mut buf).await?;
    let reply = Message::from_vec(&buf[..n])?;
    ensure!(
        reply.metadata.id == q.metadata.id
            && reply.metadata.message_type == MessageType::Response
            && reply.queries == q.queries
            && reply.metadata.response_code == ResponseCode::NoError,
        "unexpected DNS response"
    );
    if ty == RecordType::A {
        ensure!(
            reply.answers.len() == 1
                && matches!(&reply.answers[0].data,RData::A(ip) if ip.0==c.http.redirect_ip),
            "A record mismatch"
        );
    } else {
        ensure!(reply.answers.is_empty(), "expected empty DNS answer");
    }
    Ok(())
}
async fn http(addr: SocketAddr, host: &str, path: &str, target: &str) -> Result<()> {
    let mut stream = BufReader::new(TcpStream::connect(net::reachable(addr)).await?);
    stream
        .get_mut()
        .write_all(
            format!("GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n").as_bytes(),
        )
        .await?;
    let data = net::frame(&mut stream, b"\r\n\r\n", 16384).await?;
    let mut headers = [httparse::EMPTY_HEADER; 32];
    let mut r = httparse::Response::new(&mut headers);
    ensure!(
        r.parse(&data)?.is_complete()
            && r.code == Some(302)
            && r.headers
                .iter()
                .any(|h| h.name.eq_ignore_ascii_case("Location")
                    && h.value == format!("http://{target}{path}").as_bytes()),
        "redirect mismatch"
    );
    Ok(())
}
async fn check(name: &str, f: impl Future<Output = Result<()>>) -> Value {
    match timeout(Duration::from_secs(3), f).await {
        Ok(Ok(())) => json!({"name":name,"passed":true,"detail":"OK"}),
        Ok(Err(e)) => json!({"name":name,"passed":false,"detail":e.to_string()}),
        Err(_) => json!({"name":name,"passed":false,"detail":"timed out"}),
    }
}
pub async fn run(c: &Config) -> Value {
    let (a, aaaa, srv, core, legacy) = tokio::join!(
        check(
            "dns_redirect_a",
            dns(c, "xvcf1.xboxlive.com", RecordType::A)
        ),
        check(
            "dns_redirect_aaaa",
            dns(c, "xvcf1.xboxlive.com", RecordType::AAAA)
        ),
        check(
            "dns_srv_block",
            dns(c, "assets-srvs.xboxlive.com", RecordType::SRV)
        ),
        check(
            "http_redirect",
            http(
                c.http.addr(),
                "xvcf1.xboxlive.com",
                "/Z/routing/extraextralarge.txt",
                "assets1.xboxlive.cn"
            )
        ),
        check(
            "http_dlassets_redirect",
            http(
                c.http.addr(),
                "dlassets.xboxlive.com",
                "/test",
                "dlassets.xboxlive.cn"
            )
        )
    );
    let checks = [a, aaaa, srv, core, legacy];
    json!({"all_passed":checks.iter().all(|v|v["passed"]==true),"checks":checks})
}
