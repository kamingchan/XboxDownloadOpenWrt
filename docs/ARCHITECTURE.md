# XboxDownloadOpenWrt Architecture & Contract (v0.1)

This document is the **single source of truth** shared across the Rust daemon (`daemon/`), OpenWrt core package (`openwrt/xboxdownload/`), and LuCI frontend (`openwrt/luci-app-xboxdownload/`).

## 1. Core Design Principles

1. **Zero Blast Radius on Core Router Functions**:
   - Never modify `/etc/config/uhttpd` or restart `uhttpd`.
   - Never modify `table inet fw4` or SmartDNS configuration.
   - All firewall rules live inside an isolated `table inet xboxdownload`, atomically deleted on stop.
   - All `dnsmasq` integration lives in a single fragment (`xboxdownload.conf`) inside the active `dnsmasq` `conf-dir` (auto-discovered from `/var/etc/dnsmasq.conf.*` / `dhcp.@dnsmasq[].confdir`, compatible with OpenWrt 23.05 `/tmp/dnsmasq.d` and 24.10 `/tmp/dnsmasq.<cfg>.d`), matching **exact download/SRV hostnames only** (never broad wildcards like `xboxlive.com`). CDN groups (`xbox_cn1`, `xbox_cn2`, `xbox_app`) are only delegated to `xboxdownloadd` once a valid IP (or Akamai fallback IP) is configured, so unconfigured groups never hijack Windows Update or default SmartDNS/OpenClash resolution.
   - Stopping `xboxdownload` cleanly removes the `dnsmasq` fragment, alias IP, and `table inet xboxdownload`, leaving core routing, DHCP, SmartDNS, and OpenWrt LuCI intact.
2. **Full IPv6 Compatibility**:
   - LAN IPv6 stays enabled.
   - For redirected `.com` / SRV fallback domains: DNS returns `<alias_ip>` for `A`, and empty `NOERROR` for `AAAA`, `HTTPS` (Type 65), and `SVCB` (Type 64).
   - For CDN groups (`xbox_cn1`, `xbox_cn2`, `xbox_app`, `akamai`): supports `ipv4`, `ipv6`, and `dual` modes. When a group selects IPv4-only, `AAAA`/`HTTPS`/`SVCB` return empty `NOERROR` to prevent Happy Eyeballs bypass. When IPv6-only is selected, `AAAA` returns the winning IPv6 CDN node and `A`/`HTTPS`/`SVCB` return empty `NOERROR` (IPv4-only clients should use `ipv4` or `dual`).
   - Optional per-MAC `nftables` IPv6 block inside `table inet xboxdownload` (`input` + `forward` chains) serves as a last-resort fallback.
3. **Set-and-Forget Auto-Speedtest**:
   - Periodic background scheduler automatically tests candidate IPs and switches to a better node only when safe:
     - Defers if active HTTP 302 hits occurred within `defer_active_minutes`.
     - Applies hysteresis (`switch_threshold_pct`, e.g., 20%) unless current IP fails health check or drops below `min_speed_mbps`.
     - Never wipes a working IP if a scheduled run fails completely.
     - Note: `speed_mbps` / `min_speed_mbps` / `last_speed_mbps` JSON/UCI keys are measured in **MB/s** (megabytes per second, matching XboxDownload conventions).
4. **Open-Source Privacy & Offline Resilience**:
   - Bundled baseline IP lists under `/usr/share/xboxdownload/ip/` (`data/IP.*.txt`).
   - Synced upstream IP lists cached under `/var/lib/xboxdownload/` (tmpfs on OpenWrt; auto-refreshed on sync).
   - User custom IP lists persisted under `/etc/xboxdownload/` (`custom.<group>.txt`, preserved across reboots/sysupgrade via `conffiles`).
   - Configurable online sync mirrors (GitHub Raw, jsDelivr, or custom URL).
   - Zero telemetry or automatic IP upload to third-party servers. (`xbox_app` default speedtest optionally queries the upstream package URL resolver unless `custom_test_url` is set, and gracefully falls back to TCP RTT if unreachable.)

---

## 2. Exact Xbox Domain Rules (v0.1 Scope)

The rule model supports extensible groups; v0.1 enables the Xbox + PC Microsoft Store groups below.

### 2.1 Redirected Domains -> `<http_redirect_ip>` (12 domains)

DNS `A` returns `<http_redirect_ip>` (alias IP or configured target IP); `AAAA`/`HTTPS`/`SVCB` return empty `NOERROR`.

1. **Core Redirect Group -> `http://assets1.xboxlive.cn{uri}` (10 domains)**:
   - `assets1.xboxlive.com`
   - `assets2.xboxlive.com`
   - `xvcf1.xboxlive.com`
   - `xvcf2.xboxlive.com`
   - `d1.xboxlive.com`
   - `d2.xboxlive.com`
   - `5.xb.delivery.microsoft.com`
   - `assets1.xboxlive.com.edgesuite.net`
   - `cl-glcb907925.gcdn.co`
   - `fg.microsoft.map.fastly.net`
2. **Legacy `dlassets` Redirect Group -> `http://dlassets.xboxlive.cn{uri}` (2 domains)**:
   - `dlassets.xboxlive.com`
   - `dlassets2.xboxlive.com`

### 2.2 Blocked SRV Domains (4 domains)

DNS `SRV` (Type 33) queries return `NOERROR` with 0 answers (short TTL `60s`) to prevent mid-download fallback to overseas CDNs:
- `assets-srvs.xboxlive.com`
- `assets-srvs-net.trafficmanager.net`
- `assets-srvs-apac.delivery.microsoft.com`
- `assets-srvs-na.delivery.microsoft.com`

*(Non-SRV queries on these 4 domains also return empty `NOERROR` or forward safely without looping.)*

### 2.3 CDN Acceleration Groups (Speed-Tested IPs)

| Group ID | UCI Option Prefix | IP File Name | Target Hostnames | Default SpeedTest URL(s) |
| :--- | :--- | :--- | :--- | :--- |
| `xbox_cn1` | `xbox_cn1` | `IP.XboxCn1.txt` | `assets1.xboxlive.cn`, `assets2.xboxlive.cn`, `d1.xboxlive.cn`, `d2.xboxlive.cn` | `http://assets1.xboxlive.cn/Z/routing/extraextralarge.txt` (fallback: Halo/MSIXVC CN1 URLs) |
| `xbox_cn2` | `xbox_cn2` | `IP.XboxCn2.txt` | `dlassets.xboxlive.cn`, `dlassets2.xboxlive.cn` | `http://dlassets.xboxlive.cn/public/content/0044aef3-bd7e-4478-8782-6191546dcd9a/f6585e19-765c-49c3-ba77-326c3766e05a/1.0.4.0.60281fce-b9c0-49f6-a2b6-027abad22637/ACBF_1.0.4.0_x64__b6krnev7r9sf8` |
| `xbox_app` | `xbox_app` | `IP.XboxApp.txt` | `dl.delivery.mp.microsoft.com`, `tlu.dl.delivery.mp.microsoft.com`, `1d.tlu.dl.delivery.mp.microsoft.com` | Dynamic resolver (`https://xbox.skydevil.xyz/Game/GetAppPackage?WuCategoryId=986a47b3-0085-4c0c-b3b3-3b806f969b00`) or custom URL; graceful fallback to TCP:80 RTT ranking if offline |
| `akamai` | `akamai` | `IP.AkamaiV2.txt` (or `IP.Akamai.txt`) | Optional fallback/shared Akamai IP for `xbox_cn1`/`xbox_app` when user selects Akamai | `http://xvcf1.xboxlive.com/Z/routing/extraextralarge.txt` |

**Note on Akamai fallback**: If `xbox_cn1` / `xbox_cn2` / `xbox_app` has no dedicated IP configured, it forwards to upstream DNS (or uses `akamai` IP if `use_akamai_fallback` is enabled for that group).

---

## 3. Networking & OpenWrt Integration Contract

### 3.1 HTTP 302 Interception (`alias_ip` mode, default)
- LAN device: UCI `lan_device` (default `br-lan`).
- Alias IP: UCI `alias_ip` (e.g., `192.168.1.253`, empty by default until configured or auto-suggested in LuCI; init script refuses to start `alias_ip` mode if empty or invalid).
- Mount on start: `ip addr replace "${alias_ip}/32" dev "${lan_device}"`.
- Daemon HTTP bind: `${alias_ip}:${http_port}` (default `http_port = 18080`).
- Isolated `nftables` table `inet xboxdownload`:
  ```nft
  table inet xboxdownload {
      chain prerouting {
          type nat hook prerouting priority dstnat - 10; policy accept;
          iifname "<lan_device>" ip daddr <alias_ip> tcp dport 80 dnat to <alias_ip>:<http_port>
      }
      chain output {
          type nat hook output priority -100; policy accept;
          ip daddr <alias_ip> tcp dport 80 dnat to <alias_ip>:<http_port>
      }
  }
  ```
- Hotplug persistence: `/etc/hotplug.d/iface/95-xboxdownload` re-mounts `${alias_ip}/32` on `ifup`/`ifupdate` of `${lan_device}` if the interface reloads while the service is active.
- Optional per-MAC IPv6 block (if `block_ipv6_mac` is non-empty), inside the same `table inet xboxdownload`:
  ```nft
  chain input_v6_block {
      type filter hook input priority filter - 5; policy accept;
      iifname "<lan_device>" ether saddr { <mac1>, <mac2> } udp dport 547 drop
      iifname "<lan_device>" ether saddr { <mac1>, <mac2> } meta l4proto icmpv6 drop
  }
  chain forward_v6_block {
      type filter hook forward priority filter - 5; policy accept;
      iifname "<lan_device>" ether saddr { <mac1>, <mac2> } meta nfproto ipv6 drop
  }
  ```
- On stop:
  1. `nft delete table inet xboxdownload 2>/dev/null || true`
  2. `ip addr del "${alias_ip}/32" dev "${lan_device}" 2>/dev/null || true`
  3. Remove `xboxdownload.conf` from the recorded `dnsmasq` `conf-dir`(s) and restart `dnsmasq` once (only if a fragment was actually removed).

### 3.2 HTTP Custom Mode (`custom_port`)
- Does not touch `ip addr` or `nftables` DNAT.
- Daemon binds `${http_bind}:${http_port}` directly; DNS returns `${redirect_ip}` (or router LAN IP).

### 3.3 DNS Integration
- Daemon listens on UDP (and TCP) at `${dns_bind}:${dns_port}` (default `127.0.0.1:5353`).
- `dns_mode = "dnsmasq"` (default):
  - Init script auto-discovers active `dnsmasq` `conf-dir`(s) from `/var/etc/dnsmasq.conf.*` (falling back to `dhcp.@dnsmasq[].confdir` / `/tmp/dnsmasq.d`) so both OpenWrt 23.05 and 24.10 work out of the box.
  - Generates `xboxdownload.conf` containing **only exact managed hostnames** (12 redirect domains + 4 SRV domains + enabled CDN group domains that have a configured IP or active Akamai fallback), normalizing `0.0.0.0`/`::` to `127.0.0.1`/`::1`, plus `rebind-domain-ok=/<host>/` for the 12 redirected hostnames when `redirect_ip` is private:
    ```text
    server=/assets1.xboxlive.com/127.0.0.1#5353
    ...
    rebind-domain-ok=/assets1.xboxlive.com/
    ```
  - **Minimal `dnsmasq` disruption**: Because `dnsmasq` only reloads `conf-dir` on restart (not `SIGHUP`), `xboxdownload.init` compares the generated fragment against the installed file and triggers `/etc/init.d/dnsmasq restart` **only when the fragment is added, modified, or removed**. Routine IP switches (`apply-ip`) only send `SIGHUP` to flush cache without restarting `dnsmasq`.
- `dns_mode = "manual"`:
  - Does not write any `dnsmasq` fragment.
  - LuCI displays copy-pasteable SmartDNS / AdGuardHome / dnsmasq (with `rebind-domain-ok`) upstream snippets.
- **Fallback upstream**: UCI `upstream_dns` defaults to empty. At startup, init selects a usable global dnsmasq `server=` address, then a WAN nameserver from `resolv.conf.auto`, and finally `223.5.5.5:53`. LuCI displays an automatic choice while the field is empty. Selecting or entering an address saves an explicit override. Clearing the field restores automatic selection.
- **Loop prevention**: The upstream must never point to the daemon listener or, in `dnsmasq` mode, to **any local router interface IP on port 53**. A looping override prevents startup.

---

## 4. UCI Schema (`/etc/config/xboxdownload`)

```text
config main 'main'
    option enabled '0'
    option dns_mode 'dnsmasq'            # 'dnsmasq' | 'manual'
    option dns_bind '127.0.0.1'
    option dns_port '5353'
    option upstream_dns ''               # empty or absent selects the upstream automatically
    option dns_ttl '60'
    option http_mode 'alias_ip'          # 'alias_ip' | 'custom_port'
    option lan_device 'br-lan'
    option alias_ip ''                   # e.g. '192.168.1.253'
    option http_bind '0.0.0.0'           # used only in custom_port mode
    option http_port '18080'
    option redirect_ip ''                # override redirect target IP in custom_port mode
    option log_capacity '200'
    # Fixed runtime paths (not stored/editable in UCI for M8 security hardening):
    #   socket_path=/var/run/xboxdownload.sock
    #   data_dir=/usr/share/xboxdownload/ip
    #   cache_dir=/var/lib/xboxdownload
    #   custom_dir=/etc/xboxdownload
    #   apply_hook=/usr/libexec/xboxdownload/apply-ip
    # optional: list block_ipv6_mac 'aa:bb:cc:dd:ee:ff'

config group 'xbox_cn1'
    option enabled '1'
    option ip_family 'ipv4'              # 'ipv4' | 'ipv6' | 'dual'
    option selected_ipv4 ''
    option selected_ipv6 ''
    option filter_keyword ''             # e.g. '电信' / '联通' / '移动' / '广东'
    option custom_test_url ''
    option use_akamai_fallback '0'

config group 'xbox_cn2'
    option enabled '1'
    option ip_family 'ipv4'
    option selected_ipv4 ''
    option selected_ipv6 ''
    option filter_keyword ''
    option custom_test_url ''
    option use_akamai_fallback '0'

config group 'xbox_app'
    option enabled '1'
    option ip_family 'ipv4'
    option selected_ipv4 ''
    option selected_ipv6 ''
    option filter_keyword ''
    option custom_test_url ''
    option use_akamai_fallback '0'

config group 'akamai'
    option enabled '0'
    option ip_family 'ipv4'
    option selected_ipv4 ''
    option selected_ipv6 ''
    option filter_keyword ''
    option custom_test_url ''

config speedtest 'speedtest'
    option auto_enabled '1'
    option interval_hours '12'           # periodic interval in hours
    option prefer_hour '4'               # preferred local hour (0-23, -1 = anytime)
    option defer_active_minutes '15'     # skip/defer if HTTP 302 hit within last N mins
    option switch_threshold_pct '20'     # float: switch only if new IP is >=20% faster
    option min_speed_mbps '5.0'          # float (MB/s): force switch if current IP < min_speed_mbps
    option prefilter_concurrency '24'
    option prefilter_top_n '10'
    option download_concurrency '4'
    option download_range_mb '8'
    option download_timeout_secs '6'
    option tcp_timeout_ms '1500'
    option ip_sync_enabled '1'
    option ip_sync_mirror 'github'       # 'github' | 'jsdelivr' | 'custom'
    option ip_sync_custom_base ''        # e.g. 'https://my-mirror.example/IP/'
```

---

## 5. Runtime JSON Config Contract (`/var/etc/xboxdownload.json`)

The `xboxdownload.init` script renders UCI into `/var/etc/xboxdownload.json` (using `jsonfilter` / `jshn.sh` from `/usr/share/libubox/jshn.sh`) and launches `xboxdownloadd run --config /var/etc/xboxdownload.json`.

The rendered `dns.upstream` contains the resolved endpoint or the explicit override. The example shows the last-resort resolver. All fields have serde defaults in Rust so partial configs still work.

```json
{
  "dns": {
    "mode": "dnsmasq",
    "bind": "127.0.0.1",
    "port": 5353,
    "upstream": "223.5.5.5:53",
    "ttl": 60
  },
  "http": {
    "mode": "alias_ip",
    "bind": "192.168.1.253",
    "port": 18080,
    "redirect_ip": "192.168.1.253"
  },
  "runtime": {
    "socket_path": "/var/run/xboxdownload.sock",
    "data_dir": "/usr/share/xboxdownload/ip",
    "cache_dir": "/var/lib/xboxdownload",
    "custom_dir": "/etc/xboxdownload",
    "apply_hook": "/usr/libexec/xboxdownload/apply-ip",
    "log_capacity": 200
  },
  "groups": {
    "xbox_cn1": {
      "enabled": true,
      "ip_family": "ipv4",
      "selected_ipv4": "",
      "selected_ipv6": "",
      "filter_keyword": "",
      "custom_test_url": "",
      "use_akamai_fallback": false
    },
    "xbox_cn2": {
      "enabled": true,
      "ip_family": "ipv4",
      "selected_ipv4": "",
      "selected_ipv6": "",
      "filter_keyword": "",
      "custom_test_url": "",
      "use_akamai_fallback": false
    },
    "xbox_app": {
      "enabled": true,
      "ip_family": "ipv4",
      "selected_ipv4": "",
      "selected_ipv6": "",
      "filter_keyword": "",
      "custom_test_url": "",
      "use_akamai_fallback": false
    },
    "akamai": {
      "enabled": false,
      "ip_family": "ipv4",
      "selected_ipv4": "",
      "selected_ipv6": "",
      "filter_keyword": "",
      "custom_test_url": ""
    }
  },
  "speedtest": {
    "auto_enabled": true,
    "interval_hours": 12,
    "prefer_hour": 4,
    "defer_active_minutes": 15,
    "switch_threshold_pct": 20,
    "min_speed_mbps": 5.0,
    "prefilter_concurrency": 24,
    "prefilter_top_n": 10,
    "download_concurrency": 4,
    "download_range_mb": 8,
    "download_timeout_secs": 6,
    "tcp_timeout_ms": 1500,
    "ip_sync_enabled": true,
    "ip_sync_mirror": "github",
    "ip_sync_custom_base": ""
  }
}
```

### Apply-IP Hook Contract (`/usr/libexec/xboxdownload/apply-ip`)
When the daemon's auto-scheduler or `apply_ip` RPC selects a new IP, it updates its in-memory routing table and invokes (if configured & executable):
```sh
/usr/libexec/xboxdownload/apply-ip <group_id> <ipv4_or_empty> <ipv6_or_empty>
```
If the hook fails or times out, the daemon rolls back the in-memory selection to its previous values so memory and persisted UCI never diverge.

**Lock portability**: the init script and the apply hook acquire their locks with a polling loop around `flock -n <fd>` (retry once per second, give up after 10 seconds) instead of `flock -w`. BusyBox `flock` on stock OpenWrt images does not implement `-w`, and depending on the util-linux `flock` package would conflict with BusyBox on some firmware. The core package therefore has no `flock` dependency, and the release workflow does not install a `flock` feed package.

The hook script:
1. Validates `<group_id>` is in `{xbox_cn1, xbox_cn2, xbox_app, akamai}` and validates IP syntax.
2. Persists `selected_ipv4` / `selected_ipv6` via `uci set` + `uci commit xboxdownload` inside a private UCI delta directory, serialized by `/var/lock/xboxdownload-apply.lock`. If the second write or the JSON re-render fails, the previous pair is restored.
3. Refreshes `/var/etc/xboxdownload.json` (pure render without calling daemon RPC, avoiding lock contention) so an immediate `SIGHUP` or `procd` respawn reads the newly applied IP.
4. Sends `SIGHUP` to `dnsmasq` (`killall -HUP dnsmasq 2>/dev/null || true`) to flush cached DNS answers immediately without interrupting active connections. (If a previously empty CDN group just received its first IP, `procd`'s reload trigger asynchronously refreshes the `dnsmasq` fragment once.)

---

## 6. Local Control Protocol (Unix Socket `/var/run/xboxdownload.sock`)

Line-delimited JSON request/response over Unix stream socket (one JSON line sent, one JSON line returned, socket closed; permission `0600` owned by root).

### Request Envelope
```json
{"method": "<method_name>", "params": { ... }}
```

### Response Envelope
Success:
```json
{"ok": true, "data": { ... }}
```
Error:
```json
{"ok": false, "error": "human readable message"}
```

### Methods

1. **`status`** (`params: {}`)
   - Returns:
     ```json
     {
       "running": true,
       "version": "0.1.0",
       "uptime_secs": 1234,
       "dns_listen": "127.0.0.1:5353",
       "http_listen": "192.168.1.253:18080",
       "redirect_ip": "192.168.1.253",
       "dns_queries_total": 420,
       "srv_blocked_total": 18,
       "aaaa_filtered_total": 95,
       "http_redirects_total": 31,
       "last_redirect_ts": 1732590000,
       "groups": {
         "xbox_cn1": {"enabled": true, "ip_family": "ipv4", "selected_ipv4": "114.105.125.22", "selected_ipv6": "", "last_speed_mbps": 82.5, "last_tested_ts": 1732580000}
       },
       "scheduler": {
         "auto_enabled": true,
         "next_run_ts": 1732623200,
         "last_run_ts": 1732580000,
         "last_run_status": "switched",
         "last_run_message": "xbox_cn1 switched to 114.105.125.22 (82.5 MB/s)"
       }
     }
     ```
2. **`logs`** (`params: {"limit": 50}`)
   - Returns `{"entries": [{"ts": 1732590000, "client_ip": "192.168.1.105", "host": "xvcf1.xboxlive.com", "path": "/Z/routing/extraextralarge.txt", "target": "http://assets1.xboxlive.cn/Z/routing/extraextralarge.txt", "status": 302}]}`.
3. **`speedtest_start`** (`params: {"group": "xbox_cn1", "ip_family": "ipv4", "filter_keyword": "", "auto_apply": false}`)
   - Starts an async speedtest job (rejects if one is already running). Returns `{"job_id": "..."}`.
4. **`speedtest_status`** (`params: {}`)
   - Returns:
     ```json
     {
       "running": true,
       "job_id": "...",
       "group": "xbox_cn1",
       "ip_family": "ipv4",
       "stage": "downloading",
       "total_candidates": 120,
       "tested_count": 6,
       "total_to_download": 10,
       "error": null,
       "note": null,
       "results": [
         {"ip": "114.105.125.22", "family": "ipv4", "location": "中国安徽阜阳 电信", "rtt_ms": 18.4, "speed_mbps": 82.5, "http_status": 206, "ok": true}
       ]
     }
     ```
5. **`speedtest_cancel`** (`params: {}`)
   - Cancels the active speedtest job. Returns `{"cancelled": true}`.
6. **`apply_ip`** (`params: {"group": "xbox_cn1", "ipv4": "114.105.125.22", "ipv6": ""}`)
   - Applies in memory and triggers `apply_hook` (persists to UCI + refreshes runtime JSON + SIGHUP `dnsmasq`); rolls back memory if `apply_hook` fails. (`ipv4` and `ipv6` default to `""` if omitted.)
7. **`sync_ips`** (`params: {"mirror": "github", "custom_base": ""}`)
   - Downloads fresh `IP.XboxCn1.txt`, `IP.XboxCn2.txt`, `IP.XboxApp.txt`, `IP.Akamai.txt`, `IP.AkamaiV2.txt` into `cache_dir`. Returns per-file counts and status.
8. **`list_ips`** (`params: {"group": "xbox_cn1", "ip_family": "ipv4", "filter_keyword": ""}`)
   - Lists candidate IPs (merged bundled/cached + custom), total count, and raw `custom_text` from `${custom_dir}/custom.<group>.txt` (falls back to `cache_dir` when `custom_dir` is empty).
9. **`save_custom_ips`** (`params: {"group": "xbox_cn1", "text": "192.0.2.1\t(Custom)\n"}`)
   - Saves user custom IPs under `${custom_dir}/custom.<group>.txt` (falls back to `cache_dir` when `custom_dir` is empty).
10. **`self_check`** (`params: {}`)
    - Executes real end-to-end checks against local DNS & HTTP listeners AND (when active) the router's live `dnsmasq` / `alias_ip:80` path:
      - `dns_redirect_a`: queries daemon DNS for `xvcf1.xboxlive.com` A -> expects `redirect_ip`
      - `dns_redirect_aaaa`: queries daemon DNS for `xvcf1.xboxlive.com` AAAA -> expects empty `NOERROR`
      - `dns_srv_block`: queries daemon DNS for `assets-srvs.xboxlive.com` SRV -> expects empty `NOERROR`
      - `http_redirect`: sends `GET /Z/routing/extraextralarge.txt` (`Host: xvcf1.xboxlive.com`) to local HTTP listener -> expects `302` with `Location: http://assets1.xboxlive.cn/Z/routing/extraextralarge.txt`
      - `http_dlassets_redirect`: sends `GET /test` (`Host: dlassets.xboxlive.com`) -> expects `302` with `Location: http://dlassets.xboxlive.cn/test`
      - `dnsmasq_redirect_a` *(when `dns.mode == "dnsmasq"`)*: queries `127.0.0.1:53` for `xvcf1.xboxlive.com` A -> expects `redirect_ip` (catches missing `conf-dir` or DNS rebind blocking)
      - `http_alias_redirect` *(when `http.mode == "alias_ip"`)*: connects to `<redirect_ip>:80` (`Host: xvcf1.xboxlive.com`) -> expects `302` via `table inet xboxdownload` DNAT (catches dropped `/32` alias IP or broken nftables DNAT)
    - Returns `{"all_passed": true, "checks": [{"name": "...", "passed": true, "detail": "..."}]}`.
11. **`reload`** (`params: {}`)
    - Reloads runtime config file from disk without restarting the process.

---

## 7. `rpcd` ubus Contract (`luci.xboxdownload`)

The `rpcd` script `/usr/libexec/rpcd/luci.xboxdownload` exposes ubus object `luci.xboxdownload` with 13 methods:
- `status` -> proxies `status` (or returns `{"running": false}` if socket absent)
- `logs` (`limit`) -> proxies `logs`
- `speedtest_start` (`group`, `ip_family`, `filter_keyword`, `auto_apply`) -> proxies `speedtest_start` (only forwards optional fields when supplied)
- `speedtest_status` -> proxies `speedtest_status`
- `speedtest_cancel` -> proxies `speedtest_cancel`
- `apply_ip` (`group`, `ipv4`, `ipv6`) -> proxies `apply_ip` (or falls back to `apply-ip` hook if daemon stopped)
- `sync_ips` (`mirror`, `custom_base`) -> proxies `sync_ips`
- `list_ips` (`group`, `ip_family`, `filter_keyword`) -> proxies `list_ips` directly (daemon already returns `custom_text`)
- `save_custom_ips` (`group`, `text`) -> proxies `save_custom_ips`
- `self_check` -> proxies `self_check`
- `suggest_alias_ip` (`lan_device`) -> inspects `ip -4 addr show dev <lan_device>`, suggests an unused high IP in the same subnet (checked via `ip addr`, `ip neigh`, and `ping -I <lan_device> -c 1 -W 1`), returns `{"lan_ip": "192.168.1.1", "cidr": "192.168.1.1/24", "suggested_ip": "192.168.1.253", "in_use": false}`
- `suggest_upstream_dns` (`params: {}`) returns an automatic upstream preview and manual choices, for example `{"auto_address":"127.0.0.1:6053","candidates":[{"address":"127.0.0.1:6053","source":"dnsmasq_upstream","service":"SmartDNS"}]}`. The read-only `/etc/init.d/xboxdownload resolve_upstream` command computes `auto_address` using the startup rules while ignoring any saved override. If the command is unavailable or returns an invalid endpoint, rpcd uses the first non-public candidate or `223.5.5.5:53`. LuCI can also display candidates from an older rpcd that omits `auto_address`.
  - `dnsmasq_upstream`: global `server=` entries in configuration order from the active `/var/etc/dnsmasq.conf.*` (falling back to `dhcp.@dnsmasq[0].server`). Domain-scoped entries are skipped. A loopback upstream on the SmartDNS UCI port is labelled `SmartDNS`.
  - `wan_dns`: `nameserver` lines from `/tmp/resolv.conf.d/resolv.conf.auto` (or `/tmp/resolv.conf.auto`).
  - `local_service`: an enabled SmartDNS (`smartdns.@smartdns[0]`) on a non-53 port that dnsmasq does not already reference.
  - `public`: AliDNS `223.5.5.5`, DNSPod `119.29.29.29`, Cloudflare `1.1.1.1`, Google `8.8.8.8`.
  - Loop guard: any router-local, loopback or `dns_bind` address on port 53 or on the daemon `dns_port` is dropped, as are `0.0.0.0`, `::` and duplicates. IPv4-mapped IPv6 is canonicalized before comparison. Init independently checks the automatic endpoint and manual overrides against the configured listener and DNS mode at startup.
- `service_action` (`action`: `start|stop|restart|reload`) -> executes `/etc/init.d/xboxdownload <action>`

ACL (`/usr/share/rpcd/acl.d/luci-app-xboxdownload.json`, role `luci-app-xboxdownload`):
- **read**: `status`, `logs`, `speedtest_status`, `list_ips`, `self_check`, `suggest_alias_ip`, `suggest_upstream_dns`, plus UCI `xboxdownload`.
- **write**: `speedtest_start`, `speedtest_cancel`, `apply_ip`, `sync_ips`, `save_custom_ips`, `service_action`, plus UCI `xboxdownload`.

Any new method must be added in four places together: the `list` JSON and `call` dispatcher of the rpcd script, the ACL file, the LuCI caller in `xboxdownload.js` / views, and this section.

Note: `xboxdownloadd ctl --socket /var/run/xboxdownload.sock --method <m> --params '<json>'` CLI subcommand is also provided by the Rust binary so `rpcd` does NOT need `socat`, `python3`, or `nc -U` on OpenWrt! It simply invokes `xboxdownloadd ctl ...`, which works cleanly on minimal OpenWrt images with zero extra runtime dependencies!
