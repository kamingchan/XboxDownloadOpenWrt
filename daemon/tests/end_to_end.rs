use hickory_proto::{
    op::{Message, MessageType, OpCode, Query, ResponseCode},
    rr::{Name, RData, Record, RecordType as T, rdata::A},
};
use serde_json::{Value, json};
use std::{
    net::SocketAddr,
    os::unix::fs::PermissionsExt,
    path::Path,
    process::Stdio,
    sync::{
        Arc,
        atomic::{AtomicU16, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::{TcpListener, TcpStream, UdpSocket, UnixStream},
    process::{Child, Command},
    time::{sleep, timeout},
};
const BIN: &str = env!("CARGO_BIN_EXE_xboxdownloadd");
const CORE: &[&str] = &[
    "assets1.xboxlive.com",
    "assets2.xboxlive.com",
    "xvcf1.xboxlive.com",
    "xvcf2.xboxlive.com",
    "d1.xboxlive.com",
    "d2.xboxlive.com",
    "5.xb.delivery.microsoft.com",
    "assets1.xboxlive.com.edgesuite.net",
    "cl-glcb907925.gcdn.co",
    "fg.microsoft.map.fastly.net",
];
const LEGACY: &[&str] = &["dlassets.xboxlive.com", "dlassets2.xboxlive.com"];
const SRV: &[&str] = &[
    "assets-srvs.xboxlive.com",
    "assets-srvs-net.trafficmanager.net",
    "assets-srvs-apac.delivery.microsoft.com",
    "assets-srvs-na.delivery.microsoft.com",
];
struct Daemon {
    dir: tempfile::TempDir,
    config: Value,
    child: Child,
    dns: SocketAddr,
    http: SocketAddr,
}
async fn port() -> TcpListener {
    TcpListener::bind("127.0.0.1:0").await.unwrap()
}
impl Daemon {
    async fn start(extra: Value) -> Self {
        let dir = tempfile::tempdir_in("/tmp").unwrap();
        let dl = port().await;
        let hl = port().await;
        let dns = dl.local_addr().unwrap();
        let http = hl.local_addr().unwrap();
        let socket = dir.path().join("control.sock");
        let mut c = json!({"dns":{"port":dns.port(),"mode":"manual"},"http":{"port":http.port(),"redirect_ip":"192.0.2.8"},"runtime":{"socket_path":socket,"data_dir":Path::new(env!("CARGO_MANIFEST_DIR")).join("../data"),"cache_dir":dir.path(),"apply_hook":"","log_capacity":3},"speedtest":{"auto_enabled":false}});
        fn merge(base: &mut Value, v: Value) {
            if let Value::Object(obj) = v {
                for (k, val) in obj {
                    merge(&mut base[&k], val);
                }
            } else {
                *base = v;
            }
        }
        merge(&mut c, extra);
        let path = dir.path().join("config.json");
        std::fs::write(&path, c.to_string()).unwrap();
        drop((dl, hl));
        let mut child = Command::new(BIN)
            .args(["run", "--config"])
            .arg(&path)
            .kill_on_drop(true)
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        timeout(Duration::from_secs(5), async {
            loop {
                if UnixStream::connect(&socket).await.is_ok() {
                    break;
                }
                assert!(child.try_wait().unwrap().is_none(), "daemon exited early");
                sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        Self {
            dir,
            config: c,
            child,
            dns,
            http,
        }
    }
    fn socket(&self) -> &str {
        self.config["runtime"]["socket_path"].as_str().unwrap()
    }
    async fn rpc(&self, method: &str, params: Value) -> Value {
        let mut s = UnixStream::connect(self.socket()).await.unwrap();
        s.write_all(format!("{}\n", json!({"method":method,"params":params})).as_bytes())
            .await
            .unwrap();
        let mut bytes = vec![];
        timeout(Duration::from_secs(8), s.read_to_end(&mut bytes))
            .await
            .unwrap()
            .unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }
    async fn data(&self, method: &str, p: Value) -> Value {
        let r = self.rpc(method, p).await;
        assert_eq!(r["ok"], true, "{r}");
        r["data"].clone()
    }
    fn write_config(&self) {
        std::fs::write(self.dir.path().join("config.json"), self.config.to_string()).unwrap();
    }
    async fn reload(&self) {
        self.write_config();
        assert_eq!(self.data("reload", json!({})).await["reloaded"], true);
    }
    async fn signal(&self, sig: &str) {
        assert!(
            Command::new("kill")
                .arg(sig)
                .arg(self.child.id().unwrap().to_string())
                .status()
                .await
                .unwrap()
                .success()
        );
    }
    async fn stop(mut self, sig: &str) {
        self.signal(sig).await;
        assert!(
            timeout(Duration::from_secs(3), self.child.wait())
                .await
                .unwrap()
                .unwrap()
                .success()
        );
        assert!(!Path::new(self.socket()).exists());
    }
    async fn query(&self, host: &str, ty: T, tcp: bool) -> Message {
        let mut req = Message::query();
        req.queries.push(Query::query(
            Name::from_ascii(format!("{}.", host.trim_end_matches('.'))).unwrap(),
            ty,
        ));
        let bytes = req.to_vec().unwrap();
        let buf = if tcp {
            let mut s = TcpStream::connect(self.dns).await.unwrap();
            s.write_u16(bytes.len() as u16).await.unwrap();
            s.write_all(&bytes).await.unwrap();
            let mut data = vec![0; s.read_u16().await.unwrap() as usize];
            s.read_exact(&mut data).await.unwrap();
            data
        } else {
            let s = UdpSocket::bind("127.0.0.1:0").await.unwrap();
            s.send_to(&bytes, self.dns).await.unwrap();
            let mut data = vec![0; 2048];
            let (n, _) = timeout(Duration::from_secs(4), s.recv_from(&mut data))
                .await
                .unwrap()
                .unwrap();
            data.truncate(n);
            data
        };
        let r = Message::from_vec(&buf).unwrap();
        assert_eq!(r.metadata.id, req.metadata.id);
        assert_eq!(r.metadata.message_type, MessageType::Response);
        assert_eq!(r.metadata.response_code, ResponseCode::NoError);
        assert_eq!(r.queries, req.queries);
        r
    }
    async fn expected(&self, name: &str, ty: T, ip: Option<&str>) {
        let q = self.query(name, ty, false).await;
        match ip {
            Some(ip) => {
                assert_eq!(q.answers.len(), 1);
                assert_eq!(q.answers[0].data.to_string(), ip);
            }
            None => assert!(q.answers.is_empty()),
        }
    }
    async fn http(&self, req: &[u8]) -> String {
        let mut s = TcpStream::connect(self.http).await.unwrap();
        s.write_all(req).await.unwrap();
        let mut response = String::new();
        timeout(Duration::from_secs(3), s.read_to_string(&mut response))
            .await
            .unwrap()
            .unwrap();
        response
    }
}
#[tokio::test]
async fn dns_wire_rules_forwarding_and_loop_prevention() {
    let upstream = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let upstream_addr = upstream.local_addr().unwrap();
    let worker = tokio::spawn(async move {
        let mut bytes = [0; 1024];
        for _ in 0..3 {
            let (n, peer) = upstream.recv_from(&mut bytes).await.unwrap();
            let q = Message::from_vec(&bytes[..n]).unwrap();
            let mut a = Message::response(q.metadata.id, OpCode::Query);
            a.queries = q.queries;
            a.answers.push(Record::from_rdata(
                a.queries[0].name.clone(),
                30,
                RData::A(A([192, 0, 2, 99].into())),
            ));
            upstream.send_to(&a.to_vec().unwrap(), peer).await.unwrap();
        }
    });
    let mut d=Daemon::start(json!({"dns":{"upstream":upstream_addr},"groups":{"xbox_cn1":{"selected_ipv4":"192.0.2.10"},"xbox_cn2":{"ip_family":"ipv6","selected_ipv6":"2001:db8::20"},"xbox_app":{"ip_family":"dual","selected_ipv6":"2001:db8::30","selected_ipv4":"192.0.2.30"}}})).await;
    for host in CORE.iter().chain(LEGACY) {
        d.expected(host, T::A, Some("192.0.2.8")).await;
        for ty in [T::AAAA, T::HTTPS, T::SVCB] {
            d.expected(host, ty, None).await;
        }
    }
    for host in SRV {
        let r = d.query(host, T::SRV, false).await;
        assert!(r.answers.is_empty());
        assert_eq!(r.authorities[0].ttl, 60);
    }
    let r = d.query("ASSETS1.XBOXLIVE.COM.", T::A, true).await;
    assert_eq!(r.answers[0].ttl, 60);
    d.expected("assets1.xboxlive.cn", T::A, Some("192.0.2.10"))
        .await;
    for ty in [T::AAAA, T::HTTPS, T::SVCB] {
        d.expected("assets1.xboxlive.cn", ty, None).await;
    }
    d.expected("dlassets.xboxlive.cn", T::AAAA, Some("2001:db8::20"))
        .await;
    for ty in [T::A, T::HTTPS, T::SVCB] {
        d.expected("dlassets.xboxlive.cn", ty, None).await;
    }
    d.expected("dl.delivery.mp.microsoft.com", T::A, Some("192.0.2.30"))
        .await;
    d.expected(
        "dl.delivery.mp.microsoft.com",
        T::AAAA,
        Some("2001:db8::30"),
    )
    .await;
    d.data(
        "apply_ip",
        json!({"group":"xbox_app","ipv4":"","ipv6":"2001:db8::30"}),
    )
    .await;
    d.expected("dl.delivery.mp.microsoft.com", T::A, None).await;
    d.expected("other.assets1.xboxlive.com", T::A, Some("192.0.2.99"))
        .await;
    d.config["groups"]["xbox_cn1"]["enabled"] = false.into();
    d.config["groups"]["xbox_cn2"]["selected_ipv6"] = "".into();
    d.reload().await;
    for h in ["assets1.xboxlive.cn", "dlassets.xboxlive.cn"] {
        d.expected(h, T::A, Some("192.0.2.99")).await;
    }
    worker.await.unwrap();
    let stats = d.data("status", json!({})).await;
    assert_eq!(stats["srv_blocked_total"], 4);
    assert_eq!(stats["aaaa_filtered_total"], 13);
    for up in [d.dns.to_string(), "127.0.0.1:53".into()] {
        d.config["dns"]["upstream"] = up.into();
        d.config["dns"]["mode"] = "dnsmasq".into();
        d.write_config();
        assert_eq!(d.rpc("reload", json!({})).await["ok"], false);
        d.expected("assets1.xboxlive.com", T::A, Some("192.0.2.8"))
            .await;
    }
    d.stop("-TERM").await;
}
#[tokio::test]
async fn http_redirects_preserve_targets_and_reject_malformed_requests() {
    let d = Daemon::start(json!({})).await;
    for (hosts, target) in [
        (CORE, "assets1.xboxlive.cn"),
        (LEGACY, "dlassets.xboxlive.cn"),
    ] {
        for host in hosts {
            let r=d.http(format!("GET /path%2Fname?a=1&b=%20 HTTP/1.1\r\nhOsT: {}:80\r\nConnection: close\r\n\r\n",host.to_uppercase()).as_bytes()).await;
            assert!(r.starts_with("HTTP/1.1 302 Found\r\n"));
            assert!(r.contains(&format!(
                "Location: http://{target}/path%2Fname?a=1&b=%20\r\n"
            )));
        }
    }
    let piped=d.http(b"GET /first HTTP/1.1\r\nHost: dlassets.xboxlive.com\r\n\r\nGET /second HTTP/1.1\r\nHost: unlisted.example\r\nConnection: close\r\n\r\n").await;
    assert!(piped.contains("http://dlassets.xboxlive.cn/first\r\n"));
    assert!(piped.contains("HTTP/1.1 404 Not Found"));
    for request in [
        b"GET /x\r\nInjected: response HTTP/1.1\r\nHost: assets1.xboxlive.com\r\n\r\n".as_slice(),
        b"GET /x\nBAD HTTP/1.1\r\nHost: assets1.xboxlive.com\r\n\r\n",
        b"GET /a HTTP/1.1\r\nHost: assets1.xboxlive.com\r\nHost: other.example\r\n\r\n",
    ] {
        let r = d.http(request).await;
        assert!(r.contains("400 Bad Request"));
        assert!(!r.contains("Location:"));
    }
    let oversized = format!(
        "GET /{} HTTP/1.1\r\nHost: assets1.xboxlive.com\r\n\r\n",
        "a".repeat(16384)
    );
    assert!(
        d.http(oversized.as_bytes())
            .await
            .starts_with("HTTP/1.1 400")
    );
    let stats = d.data("status", json!({})).await;
    assert_eq!(stats["http_redirects_total"], 13);
    assert!(stats["last_redirect_ts"].as_u64().unwrap() > 0);
    let log = d.data("logs", json!({"limit":99})).await;
    assert_eq!(log["entries"].as_array().unwrap().len(), 3);
    assert_eq!(log["entries"][0]["status"], 404);
    assert_eq!(log["entries"][1]["path"], "/first");
    d.stop("-INT").await;
}
#[tokio::test]
async fn control_cli_self_check_reload_and_shutdown() {
    let mut d = Daemon::start(json!({"groups":{"xbox_cn2":{"enabled":false}}})).await;
    let perms = std::fs::metadata(d.socket()).unwrap().permissions().mode();
    assert_eq!(perms & 0o777, 0o600);
    let run =
        |args: Vec<String>| async move { Command::new(BIN).args(args).output().await.unwrap() };
    let reply = run(vec![
        "ctl".into(),
        "--socket".into(),
        d.socket().into(),
        "--method".into(),
        "self_check".into(),
    ])
    .await;
    assert!(reply.status.success());
    let r: Value = serde_json::from_slice(&reply.stdout).unwrap();
    assert_eq!(r["data"]["all_passed"], true, "{r}");
    assert_eq!(r["data"]["checks"].as_array().unwrap().len(), 5);
    for args in [
        vec!["ctl", "--socket", d.socket(), "--method", "missing"],
        vec!["ctl", "--method", "status"],
        vec![
            "ctl",
            "--socket",
            d.socket(),
            "--method",
            "status",
            "--params",
            "{",
        ],
        vec!["ctl", "--socket", "/missing.sock", "--method", "status"],
    ] {
        let out = run(args.into_iter().map(str::to_owned).collect()).await;
        assert!(!out.status.success());
        assert_eq!(
            serde_json::from_slice::<Value>(&out.stdout).unwrap()["ok"],
            false
        );
    }
    let result = Command::new(BIN)
        .arg("print-dnsmasq")
        .arg("--config")
        .arg(d.dir.path().join("config.json"))
        .output()
        .await
        .unwrap();
    let text = String::from_utf8(result.stdout).unwrap();
    assert_eq!(text.lines().count(), 23);
    for host in CORE.iter().chain(LEGACY).chain(SRV) {
        assert!(text.contains(&format!("server=/{host}/127.0.0.1#{}\n", d.dns.port())));
    }
    assert!(!text.contains("server=/xboxlive.com/"));
    assert!(!text.contains("server=/dlassets.xboxlive.cn/"));
    d.config["http"]["redirect_ip"] = "198.51.100.8".into();
    d.write_config();
    d.signal("-HUP").await;
    timeout(Duration::from_secs(3), async {
        while d.data("status", json!({})).await["redirect_ip"] != "198.51.100.8" {
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    d.expected("d1.xboxlive.com", T::A, Some("198.51.100.8"))
        .await;
    let old = d.http;
    let new = port().await;
    d.http = new.local_addr().unwrap();
    drop(new);
    d.config["http"]["port"] = d.http.port().into();
    d.reload().await;
    assert_eq!(d.data("self_check", json!({})).await["all_passed"], true);
    assert!(TcpStream::connect(old).await.is_err());
    d.stop("-TERM").await;
}
#[tokio::test]
async fn ip_lists_merge_filter_and_validate_mirror_before_replace() {
    let d = Daemon::start(json!({})).await;
    let p = |id: &str, family: &str, keyword: &str| json!({"group":id,"ip_family":family,"filter_keyword":keyword});
    for (id, keyword, v4, v6) in [
        ("xbox_cn1", "电信", "114.105.125.22", "240e:95a:4000::3013"),
        ("akamai", "日本", "", "2600:140b:2c00:be::172e:9ba8"),
    ] {
        let list = d.data("list_ips", p(id, "dual", keyword)).await;
        let ips = list["ips"].as_array().unwrap();
        assert!(ips.iter().any(|e| e["ip"] == v6));
        assert!(
            ips.iter()
                .all(|e| e["location"].as_str().unwrap().contains(keyword))
        );
        if !v4.is_empty() {
            assert!(ips.iter().any(|e| e["ip"] == v4));
        }
        let v4only = d.data("list_ips", p(id, "ipv4", "")).await;
        assert!(v4only["total"].as_u64().unwrap() > 0);
        assert!(
            v4only["ips"]
                .as_array()
                .unwrap()
                .iter()
                .all(|e| e["family"] == "ipv4")
        );
    }
    let custom =
        "192.0.2.10  (广东 电信)\r\n2001:db8::10\t(Custom)\r\n114.105.125.22\t(duplicate)\n";
    let saved = d
        .data("save_custom_ips", json!({"group":"xbox_cn1","text":custom}))
        .await;
    assert_eq!(saved["count"], 3);
    let l = d.data("list_ips", p("xbox_cn1", "dual", "")).await;
    assert_eq!(l["custom_text"], custom);
    let ips = l["ips"].as_array().unwrap();
    assert_eq!(
        ips.iter().filter(|e| e["ip"] == "114.105.125.22").count(),
        1
    );
    assert!(ips.iter().any(|e| e["location"] == "广东 电信"));
    assert_eq!(
        d.rpc(
            "save_custom_ips",
            json!({"group":"xbox_cn1","text":"invalid ip"})
        )
        .await["ok"],
        false
    );
    assert_eq!(
        d.rpc(
            "save_custom_ips",
            json!({"group":"../outside","text":"192.0.2.1"})
        )
        .await["ok"],
        false
    );
    let cached = "XboxCn1\n192.0.2.90 (local cache)\n";
    let dest = d.dir.path().join("IP.XboxCn1.txt");
    std::fs::write(&dest, cached).unwrap();
    let l = d.data("list_ips", p("xbox_cn1", "dual", "")).await;
    assert_eq!(l["total"], 4);
    let mirror = port().await;
    let addr = mirror.local_addr().unwrap();
    let serve = tokio::spawn(async move {
        for _ in 0..5 {
            let (s, _) = mirror.accept().await.unwrap();
            let mut r = BufReader::new(s);
            let mut first = String::new();
            r.read_line(&mut first).await.unwrap();
            let name = first
                .split_whitespace()
                .nth(1)
                .unwrap()
                .trim_start_matches("/db/IP.")
                .trim_end_matches(".txt");
            let body = match name {
                "XboxCn1" => "wrong file\n192.0.2.91".to_owned(),
                "Akamai" => "Akamai\n\n".into(),
                _ => format!("{name}\r\n\r\n192.0.2.91\t(mirror)\r\n"),
            };
            r.get_mut()
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
        }
    });
    let sync = d
        .data(
            "sync_ips",
            json!({"mirror":"custom","custom_base":format!("http://{addr}/db")}),
        )
        .await;
    serve.await.unwrap();
    assert_eq!(sync["all_synced"], false);
    assert_eq!(sync["files"].as_object().unwrap().len(), 5);
    assert_eq!(sync["files"]["IP.XboxCn1.txt"]["ok"], false);
    assert_eq!(sync["files"]["IP.Akamai.txt"]["ok"], false);
    assert_eq!(sync["files"]["IP.XboxApp.txt"]["count"], 1);
    assert_eq!(std::fs::read_to_string(dest).unwrap(), cached);
    assert_eq!(
        d.data("list_ips", p("xbox_app", "dual", "")).await["ips"][0]["ip"],
        "192.0.2.91"
    );
    d.stop("-TERM").await;
}
// The prefilter connects and closes immediately. Only actual test requests receive a body.
async fn mock_node(listener: TcpListener, status: Arc<AtomicU16>, delay: Duration) {
    let port = listener.local_addr().unwrap().port();
    loop {
        let (socket, _) = listener.accept().await.unwrap();
        let status = status.clone();
        tokio::spawn(async move {
            let mut r = BufReader::new(socket);
            let mut headers = String::new();
            loop {
                let mut line = String::new();
                if r.read_line(&mut line).await.unwrap_or(0) == 0 {
                    return;
                }
                if line == "\r\n" {
                    break;
                }
                headers.push_str(&line.to_lowercase());
            }
            assert!(
                headers.starts_with("get /probe?q=1 http/1.1\r\n"),
                "{headers}"
            );
            assert!(headers.contains(&format!("host: test.invalid:{port}\r\n")));
            assert!(headers.contains("user-agent: xboxdownload\r\n"));
            assert!(headers.contains("range: bytes=0-1048575\r\n"));
            let status = status.load(Ordering::Relaxed);
            if status == 0 {
                sleep(Duration::from_secs(5)).await;
                return;
            }
            let (code, len) = if status == 204 {
                (200, 0)
            } else {
                (status, 65536)
            };
            let head = format!(
                "HTTP/1.1 {code} Result\r\nContent-Length: {len}\r\nConnection: close\r\n\r\n"
            );
            r.get_mut().write_all(head.as_bytes()).await.unwrap();
            sleep(delay).await;
            let _ = r.get_mut().write_all(&vec![b'x'; len]).await;
        });
    }
}
async fn finished(d: &Daemon) -> Value {
    timeout(Duration::from_secs(4), async {
        loop {
            let job = d.data("speedtest_status", json!({})).await;
            if job["running"] == false {
                return job;
            }
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap()
}
#[tokio::test]
async fn speedtest_ranks_body_bytes_rejects_error_pages_and_cancels() {
    let ipv4 = port().await;
    let port = ipv4.local_addr().unwrap().port();
    let ipv6 = TcpListener::bind((std::net::Ipv6Addr::LOCALHOST, port))
        .await
        .unwrap();
    let v4 = Arc::new(AtomicU16::new(206));
    let v6 = Arc::new(AtomicU16::new(200));
    tokio::spawn(mock_node(ipv4, v4.clone(), Duration::from_millis(15)));
    tokio::spawn(mock_node(ipv6, v6.clone(), Duration::from_millis(170)));
    let d=Daemon::start(json!({"groups":{"xbox_cn1":{"ip_family":"dual","custom_test_url":format!("http://test.invalid:{port}/probe?q=1")}},"speedtest":{"download_range_mb":1,"download_timeout_secs":3}})).await;
    std::fs::write(
        d.dir.path().join("IP.XboxCn1.txt"),
        "XboxCn1\n127.0.0.1 (fast)\n::1 (slow)\n",
    )
    .unwrap();
    let opt = json!({"group":"xbox_cn1","ip_family":"dual","auto_apply":true});
    d.data("speedtest_start", opt.clone()).await;
    let result = finished(&d).await;
    assert_eq!(result["stage"], "completed", "{result}");
    let rows = result["results"].as_array().unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["ip"], "127.0.0.1");
    assert!(rows[0]["speed_mbps"].as_f64().unwrap() > rows[1]["speed_mbps"].as_f64().unwrap() * 2.);
    assert_eq!(rows[0]["http_status"], 206);
    assert_eq!(rows[1]["http_status"], 200);
    v4.store(403, Ordering::Relaxed);
    v6.store(500, Ordering::Relaxed);
    d.data("speedtest_start", opt.clone()).await;
    let job = finished(&d).await;
    assert_eq!(job["stage"], "failed");
    assert!(
        job["results"]
            .as_array()
            .unwrap()
            .iter()
            .all(|m| m["ok"] == false
                && (m["http_status"] == 403 || m["http_status"] == 500)
                && m["speed_mbps"] == 0.)
    );
    let g = &d.data("status", json!({})).await["groups"]["xbox_cn1"];
    assert_eq!(g["selected_ipv4"], "127.0.0.1");
    assert_eq!(g["selected_ipv6"], "::1");
    v4.store(204, Ordering::Relaxed);
    d.data(
        "speedtest_start",
        json!({"group":"xbox_cn1","ip_family":"ipv4"}),
    )
    .await;
    let job = finished(&d).await;
    assert_eq!(job["results"][0]["http_status"], 200);
    assert_eq!(job["results"][0]["ok"], false);
    v4.store(0, Ordering::Relaxed);
    d.data("speedtest_start", json!({"group":"xbox_cn1"})).await;
    timeout(Duration::from_secs(2), async {
        loop {
            if d.data("speedtest_status", json!({})).await["stage"] == "downloading" {
                break;
            }
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(d.rpc("speedtest_start", opt).await["ok"], false);
    assert_eq!(
        d.data("speedtest_cancel", json!({})).await["cancelled"],
        true
    );
    assert_eq!(finished(&d).await["stage"], "cancelled");
    d.stop("-TERM").await;
}

#[tokio::test]
async fn apply_hook_receives_both_families_and_invalid_updates_are_atomic() {
    let mut d = Daemon::start(json!({})).await;
    let hook = d.dir.path().join("apply-ip");
    let receipt = d.dir.path().join("args");
    std::fs::write(
        &hook,
        format!(
            "#!/bin/sh\nprintf '%s\\n' \"$@\" > '{}'\n",
            receipt.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o700)).unwrap();
    d.config["runtime"]["apply_hook"] = json!(hook);
    d.config["groups"]["xbox_cn1"]["ip_family"] = "dual".into();
    d.reload().await;
    let valid = json!({"group":"xbox_cn1","ipv4":"192.0.2.41","ipv6":"2001:db8::41"});
    d.data("apply_ip", valid.clone()).await;
    let args = "xbox_cn1\n192.0.2.41\n2001:db8::41\n";
    assert_eq!(std::fs::read_to_string(&receipt).unwrap(), args);
    let mut invalid = valid;
    invalid["ipv4"] = "arbitrary text".into();
    assert_eq!(d.rpc("apply_ip", invalid).await["ok"], false);
    d.expected("assets1.xboxlive.cn", T::A, Some("192.0.2.41"))
        .await;
    d.expected("assets1.xboxlive.cn", T::AAAA, Some("2001:db8::41"))
        .await;
    assert_eq!(std::fs::read_to_string(receipt).unwrap(), args);
    d.stop("-TERM").await;
}
