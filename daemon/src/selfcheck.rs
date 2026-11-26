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
async fn dns(c: &Config, dest: SocketAddr, name: &str, ty: RecordType) -> Result<()> {
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
    let listener = net::reachable(c.dns.addr());
    const HOST: &str = "xvcf1.xboxlive.com";
    const PATH: &str = "/Z/routing/extraextralarge.txt";
    const TARGET: &str = "assets1.xboxlive.cn";
    let (a, aaaa, srv, core, legacy, dnsmasq, alias) = tokio::join!(
        check("dns_redirect_a", dns(c, listener, HOST, RecordType::A)),
        check(
            "dns_redirect_aaaa",
            dns(c, listener, HOST, RecordType::AAAA)
        ),
        check(
            "dns_srv_block",
            dns(c, listener, "assets-srvs.xboxlive.com", RecordType::SRV)
        ),
        check("http_redirect", http(c.http.addr(), HOST, PATH, TARGET)),
        check(
            "http_dlassets_redirect",
            http(
                c.http.addr(),
                "dlassets.xboxlive.com",
                "/test",
                "dlassets.xboxlive.cn"
            )
        ),
        async {
            if c.dns.mode == "dnsmasq" {
                let dest = SocketAddr::from(([127, 0, 0, 1], 53));
                Some(check("dnsmasq_redirect_a", dns(c, dest, HOST, RecordType::A)).await)
            } else {
                None
            }
        },
        async {
            if c.http.mode == "alias_ip" {
                let dest = SocketAddr::from((c.http.redirect_ip, 80));
                Some(check("http_alias_redirect", http(dest, HOST, PATH, TARGET)).await)
            } else {
                None
            }
        },
    );
    let mut checks = vec![a, aaaa, srv, core, legacy];
    checks.extend(dnsmasq);
    checks.extend(alias);
    json!({"all_passed":checks.iter().all(|v|v["passed"]==true),"checks":checks})
}
