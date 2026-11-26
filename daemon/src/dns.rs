use crate::{
    config::{Config, Dns},
    net, rules,
    state::Shared,
};
use anyhow::{Result, ensure};
use hickory_proto::{
    op::{Message, MessageType, OpCode, ResponseCode},
    rr::{
        DNSClass, Name, RData, Record, RecordType as T,
        rdata::{A, AAAA, SOA},
    },
};
use std::{
    net::{IpAddr, SocketAddr},
    sync::{Arc, atomic::Ordering},
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream, UdpSocket},
    sync::Semaphore,
    time::timeout,
};
const DEADLINE: Duration = Duration::from_secs(3);
pub fn loops(c: &Dns) -> bool {
    let up = c.upstream.ip().to_canonical();
    let bind = c.bind.to_canonical();
    if c.upstream.port() == 0
        || up.is_unspecified()
        || (c.mode == "dnsmasq" && c.upstream.port() == 53 && up.is_loopback())
    {
        return true;
    }
    c.port == c.upstream.port()
        && (bind == up
            || (bind.is_unspecified()
                && (up.is_loopback()
                    || if_addrs::get_if_addrs()
                        .map(|a| a.iter().any(|i| i.ip().to_canonical() == up))
                        .unwrap_or(true))))
}
enum Answer {
    Address(IpAddr),
    Empty,
    Forward,
}
fn policy(c: &Config, host: &str, ty: T) -> Answer {
    if rules::target(host).is_some() {
        return match ty {
            T::A => Answer::Address(c.http.redirect_ip.into()),
            T::AAAA | T::SVCB | T::HTTPS => Answer::Empty,
            _ => Answer::Forward,
        };
    }
    if rules::SRV.contains(&host) {
        return Answer::Empty;
    }
    let Some(group) = rules::GROUPS
        .iter()
        .find(|r| r.hosts.contains(&host))
        .and_then(|r| c.groups.get(r.id))
        .filter(|g| g.enabled)
    else {
        return Answer::Forward;
    };
    let akamai = c
        .groups
        .get("akamai")
        .filter(|g| g.enabled && group.use_akamai_fallback);
    let mut addresses: Vec<_> = group.selected().collect();
    if addresses.is_empty()
        && let Some(g) = akamai
    {
        addresses.extend(g.selected().filter(|ip| group.ip_family.allows(*ip)));
    }
    if addresses.is_empty() {
        return Answer::Forward;
    }
    match ty {
        T::A | T::AAAA => addresses
            .into_iter()
            .find(|ip| ip.is_ipv4() == (ty == T::A))
            .map(Answer::Address)
            .unwrap_or(Answer::Empty),
        T::SVCB | T::HTTPS => Answer::Empty,
        _ => Answer::Forward,
    }
}
async fn forward(bytes: &[u8], up: SocketAddr, tcp: bool, request: &Message) -> Result<Vec<u8>> {
    let data = if tcp {
        let mut s = TcpStream::connect(up).await?;
        s.write_u16(bytes.len() as u16).await?;
        s.write_all(bytes).await?;
        let mut data = vec![0; s.read_u16().await? as usize];
        s.read_exact(&mut data).await?;
        data
    } else {
        let socket = UdpSocket::bind(if up.is_ipv4() { "0.0.0.0:0" } else { "[::]:0" }).await?;
        socket.connect(up).await?;
        socket.send(bytes).await?;
        let mut data = vec![0; 65535];
        let n = socket.recv(&mut data).await?;
        data.truncate(n);
        data
    };
    let answer = Message::from_vec(&data)?;
    ensure!(
        answer.metadata.message_type == MessageType::Response
            && answer.metadata.id == request.metadata.id
            && answer.queries == request.queries,
        "upstream response mismatch"
    );
    Ok(data)
}
async fn reply(s: &Shared, bytes: &[u8], tcp: bool) -> Result<Vec<u8>> {
    let request = Message::from_vec(bytes)?;
    let m = &request.metadata;
    ensure!(m.message_type == MessageType::Query, "expected DNS query");
    s.stats.dns.fetch_add(1, Ordering::Relaxed);
    let mut answer = Message::response(m.id, m.op_code);
    answer.metadata.recursion_desired = m.recursion_desired;
    answer.metadata.recursion_available = true;
    answer.queries = request.queries.clone();
    let c = s.snapshot();
    if let [q] = request.queries.as_slice()
        && m.op_code == OpCode::Query
        && q.query_class == DNSClass::IN
    {
        let name = q.name.to_ascii().to_ascii_lowercase();
        let host = name.trim_end_matches('.');
        let ty = q.query_type;
        match policy(&c, host, ty) {
            Answer::Address(ip) => {
                let data = match ip {
                    IpAddr::V4(v) => RData::A(A(v)),
                    IpAddr::V6(v) => RData::AAAA(AAAA(v)),
                };
                answer
                    .answers
                    .push(Record::from_rdata(q.name.clone(), c.dns.ttl, data));
            }
            Answer::Empty => {
                if ty == T::AAAA {
                    s.stats.aaaa.fetch_add(1, Ordering::Relaxed);
                }
                if ty == T::SRV && rules::SRV.contains(&host) {
                    s.stats.srv.fetch_add(1, Ordering::Relaxed);
                }
                let ttl = if rules::SRV.contains(&host) {
                    60
                } else {
                    c.dns.ttl
                };
                let soa = SOA::new(Name::root(), Name::root(), 1, 60, 60, 60, ttl);
                answer
                    .authorities
                    .push(Record::from_rdata(q.name.clone(), ttl, RData::SOA(soa)));
            }
            Answer::Forward => {
                if !loops(&c.dns)
                    && let Ok(Ok(data)) =
                        timeout(DEADLINE, forward(bytes, c.dns.upstream, tcp, &request)).await
                {
                    return Ok(data);
                }
                answer.metadata.response_code = ResponseCode::ServFail;
            }
        }
    } else {
        answer.metadata.response_code = ResponseCode::FormErr;
    }
    Ok(answer.to_vec()?)
}
pub async fn udp(s: Shared, socket: UdpSocket) {
    let socket = Arc::new(socket);
    let slots = Arc::new(Semaphore::new(128));
    let mut buffer = vec![0; 65535];
    loop {
        let Ok((n, addr)) = socket.recv_from(&mut buffer).await else {
            continue;
        };
        let Ok(permit) = slots.clone().try_acquire_owned() else {
            continue;
        };
        let packet = buffer[..n].to_vec();
        let socket = socket.clone();
        let s = s.clone();
        tokio::spawn(async move {
            let _permit = permit;
            if let Ok(data) = reply(&s, &packet, false).await {
                let _ = socket.send_to(&data, addr).await;
            }
        });
    }
}
pub async fn tcp(s: Shared, lis: TcpListener) {
    net::serve_tcp(lis, move |mut stream, _| {
        let s = s.clone();
        async move {
            loop {
                timeout(Duration::from_secs(15), async {
                    let len = stream.read_u16().await?;
                    let mut packet = vec![0; len as usize];
                    stream.read_exact(&mut packet).await?;
                    let data = reply(&s, &packet, true).await?;
                    stream.write_u16(data.len() as u16).await?;
                    stream.write_all(&data).await?;
                    Ok::<_, anyhow::Error>(())
                })
                .await??;
            }
        }
    })
    .await
}
