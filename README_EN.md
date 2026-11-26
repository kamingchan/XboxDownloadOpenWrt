# XboxDownloadOpenWrt

**An OpenWrt LuCI plugin that speeds up Xbox and Microsoft Store downloads: the router tests CDN nodes, picks the fastest one and forwards only the download hostnames, with no setup on your consoles or PCs.**

[简体中文](README.md) · [English](README_EN.md) · [Architecture & API contract](docs/ARCHITECTURE.md) · [Deployment cases](docs/CASES.md) · [Development guide](docs/DEVELOPMENT.md) · [Contributing](CONTRIBUTING.md)

![License: GPL-3.0-or-later](https://img.shields.io/badge/license-GPL--3.0--or--later-blue)
![OpenWrt 23.05 | 24.10](https://img.shields.io/badge/OpenWrt-23.05%20%7C%2024.10-00B5E2?logo=openwrt&logoColor=white)
![Rust](https://img.shields.io/badge/daemon-Rust-orange?logo=rust)
![LuCI](https://img.shields.io/badge/UI-LuCI%20JS-2c3e50)
![Zero Telemetry](https://img.shields.io/badge/telemetry-zero-brightgreen)

XboxDownloadOpenWrt brings the domain rules, speed-testing approach and IP databases of [XboxDownload](https://github.com/skydevil88/XboxDownload) to an OpenWrt router, as a standalone Rust daemon plus a LuCI app. Downloads from Xbox Series X|S, Xbox One, the Xbox App on Windows, Microsoft Store and PC Game Pass are steered to the fastest measured CDN node in mainland China, as long as the device uses the router for DNS. Your existing dnsmasq, SmartDNS, AdGuardHome, fw4 firewall and uhttpd web interface stay untouched.

## Highlights

- **One router for the whole household**: no DNS changes on consoles or PCs and no PC that must stay on.
- **Lightweight**: a single statically linked Rust binary (about 2.2 MB for the aarch64 release) using pure-Rust `rustls`, with no OpenSSL dependency.
- **Non-invasive**: does not take port 80 from uhttpd and does not rewrite fw4, SmartDNS or AdGuardHome configuration. Only exact Xbox download hostnames are forwarded.
- **Automatic node selection**: two-stage speed test (TCP pre-filter, then HTTP Range download), scheduled re-tests during quiet periods, switches only when clearly faster and keeps the current node if a run fails.
- **Visual control**: a responsive LuCI panel for desktop and phone with upstream DNS detection, free LAN address suggestion, redirect logs and a one-click 7-step end-to-end check.
- **Privacy**: zero telemetry. Speed-test results, node IPs and access logs are never reported anywhere.

## Contents

- [What it does](#what-it-does)
- [Comparison with the original approaches](#comparison-with-the-original-approaches)
- [How it works](#how-it-works)
- [Compatibility and architectures](#compatibility-and-architectures)
- [Installation](#installation)
- [First-time setup](#first-time-setup)
- [Speed tests and daily use](#speed-tests-and-daily-use)
- [Quick Deployment for AI Agents](#quick-deployment-for-ai-agents)
- [Troubleshooting](#troubleshooting)
- [Disable and uninstall](#disable-and-uninstall)
- [Privacy](#privacy)
- [FAQ](#faq)
- [Documentation map](#documentation-map)
- [Credits and license](#credits-and-license)

## What it does

The default Xbox CDN routing often sends users in mainland China to congested or overseas nodes. XboxDownloadOpenWrt does three things on the router:

1. **DNS forwarding**: dnsmasq forwards only the exact Xbox and Microsoft Store download hostnames to the local daemon. Everything else goes to your existing upstream (SmartDNS, AdGuardHome, ISP DNS and so on).
2. **HTTP 302 redirect**: HTTP requests for international download hosts such as `assets1.xboxlive.com` are redirected to the matching `.cn` download host, preserving the path and query string.
3. **CDN node selection**: candidate nodes in four groups (CN1, CN2, Xbox App, Akamai) are speed-tested and the `.cn` download hosts resolve to the fastest one. Game files flow directly from the CDN to the console or PC, not through the router.

Actual gains depend on your ISP, regional routing and time of day.

## Comparison with the original approaches

| | Original XboxDownload (desktop) | Hand-written hosts / dnsmasq rules on OpenWrt | **XboxDownloadOpenWrt** |
| :--- | :--- | :--- | :--- |
| Where it runs | An always-on computer; consoles point their DNS at it | Router | Router; every LAN device benefits |
| Console / PC setup | Change DNS or hosts on each device | None | None, just use the router DNS |
| Node selection | Manual speed test in the desktop app | Find IPs by hand and update them manually | Automatic background tests with hysteresis and failure fallback |
| Port 80 and web UI | Occupies port 80 on the computer | You must handle the HTTP redirect yourself | LAN `/32` alias IP + dedicated nftables DNAT; uhttpd keeps port 80 |
| Impact on the network | Depends on the computer's DNS service | Broad rules can break Windows Update and other traffic | Exact hostnames only; groups without a selected IP are not taken over, so Windows Update keeps using the existing resolver chain |
| SmartDNS / AdGuardHome / OpenClash | Not applicable | Manual coordination | Their configuration is never modified; the plugin follows local SmartDNS when dnsmasq uses it as its upstream |
| IPv6 | Depends on settings | AAAA must be blocked by hand | IPv4 / IPv6 / dual stack per group; records that could bypass the chosen node are suppressed |
| Footprint | Resident desktop application | No extra process | About 2.2 MB static Rust binary, near-zero idle CPU |
| Verification | Manual observation | Manual `nslookup` | One-click 7-step live-path self-check plus redirect log |

The original XboxDownload offers a broader toolset (for example more platforms and desktop utilities). This project focuses on one scenario, router-side download forwarding and node selection for Xbox and Microsoft Store, and is grateful for the original rules and data.

### Advantages in detail

<details>
<summary><b>A single static Rust daemon</b></summary>

- One `xboxdownloadd` binary provides DNS, HTTP 302, speed tests, scheduling and the local control interface. It is statically linked against musl with no runtime library dependencies.
- HTTPS uses `rustls` with bundled WebPKI roots, without OpenSSL. `rpcd` calls `xboxdownloadd ctl` directly, so socat, python3 or `nc -U` are not needed.
- Release builds use `opt-level = "z"`, LTO and strip to suit routers with small flash.

</details>

<details>
<summary><b>Non-invasive router integration</b></summary>

- **Port 80 stays free**: by default a `/32` alias IP is added to the LAN interface, and only traffic to that address on port 80 is DNATed to the daemon on port 18080, inside a dedicated `table inet xboxdownload`. uhttpd and the LuCI login page are unaffected.
- **Existing configuration is left alone**: `/etc/config/uhttpd`, the fw4 main table, SmartDNS and AdGuardHome configuration are never rewritten. dnsmasq integration is a single temporary fragment, `xboxdownload.conf`.
- **Exact matching**: only 12 download redirect hosts, 4 SRV fallback hosts and the `.cn` download hosts of groups with a selected node are matched, never broad wildcards such as `xboxlive.com`.
- **No premature takeover**: a CDN group is forwarded only after it has a usable IP (or an enabled Akamai fallback). Until then, Windows Update and other requests keep using your SmartDNS or OpenClash resolver chain.
- **Clean exit**: stopping the service removes the alias IP, its nftables table and the dnsmasq fragment. dnsmasq restarts once only when the fragment is added, changed or removed; routine node switches just send `SIGHUP` to flush the cache.

</details>

<details>
<summary><b>Automation and user experience</b></summary>

- **Responsive LuCI panel**: Overview, Node speed test and Settings pages that work in desktop and phone browsers, with a Simplified Chinese translation.
- **Automatic upstream DNS**: `upstream_dns` is empty by default. At startup, the service uses a global dnsmasq `server=` entry (such as local SmartDNS on a non-53 port), then a WAN DNS server, and finally `223.5.5.5:53` if neither provides a usable address. LuCI shows the detected upstream and accepts a manual override.
- **Free LAN address suggestion**: proposes an unused high address in the LAN subnet for the alias IP, checked against `ip addr`, `ip neigh` and ping.
- **Two-stage speed test**: concurrent TCP latency pre-filtering, then real HTTP Range downloads from the shortlisted nodes. The current node is measured in the same batch as the candidates so results are comparable. IPv4, IPv6 and dual stack are supported, with filters by ISP, region or note.
- **Smart scheduled switching**: every 12 hours by default, preferably at 04:00 local time; deferred if downloads happened in the last 15 minutes; a new node must be at least 20% faster; a failed node or one below 5 MB/s is replaced early; a fully failed run keeps the current node.
- **7-step live-path self-check**: covers daemon DNS (A / AAAA / SRV), both HTTP 302 redirect types, real resolution through dnsmasq on port 53 and the real redirect through the alias IP DNAT on port 80.

</details>

<details>
<summary><b>Security, privacy and compatibility</b></summary>

- **Zero telemetry**: nothing is collected or reported. See [Privacy](#privacy).
- **Hard-coded runtime paths**: the control socket, data, cache and hook paths are fixed in the binary and init script rather than read from UCI, so LuCI users cannot redirect them to escalate privileges.
- **DNS loop protection on every interface**: at start-up the upstream is compared with all router interface addresses (including IPv4-mapped IPv6 spellings), and looping configurations are rejected.
- **BusyBox compatible**: locks poll `flock -n`, which BusyBox `flock` supports, so the util-linux `flock` package is not required and no package conflict arises.
- **Multiple firmware versions**: the active dnsmasq `conf-dir` is discovered automatically on OpenWrt 23.05 (`/tmp/dnsmasq.d`) and 24.10 (`/tmp/dnsmasq.<cfg>.d`), including multiple dnsmasq instances. Works on official OpenWrt and derivatives such as FriendlyWrt and ImmortalWrt.
- **Settings survive upgrades**: custom IP lists live in `/etc/xboxdownload/`, persist across reboots and are registered for sysupgrade backups by the IPK.

</details>

## How it works

```text
Xbox / PC ──DNS──► dnsmasq ──ordinary domains──► existing SmartDNS / AdGuardHome / upstream
                      └──download and SRV names──► xboxdownloadd :5353

Xbox / PC ──HTTP:80──► LAN alias IP ──dedicated DNAT──► xboxdownloadd HTTP :18080
          ◄──302 Location: http://assets1.xboxlive.cn/original-path──┘
          └──resolve .cn download host──► speed-tested CDN node ──► direct download
```

- Redirect hosts answer A with the alias IP and empty NOERROR for AAAA, HTTPS and SVCB. The designated SRV fallback hosts receive empty answers so downloads do not fall back to overseas nodes mid-way.
- Legacy `dlassets` requests are redirected to `dlassets.xboxlive.cn`. The Xbox App uses its own download hosts and has its own group.
- Four node groups: CN1, CN2, Xbox App and Akamai (Akamai is disabled by default and can serve as a fallback for the other three).
- Optional manual DNS mode: no dnsmasq fragment is written; LuCI generates SmartDNS, AdGuardHome or dnsmasq rules to copy. Copy them again after a group first gets a node or the set of forwarded groups changes.
- Optional per-MAC IPv6 block: use it only when a specific device keeps using cached IPv6 routes.

The full domain list, UCI options, runtime JSON and ubus API are documented in the [architecture & API contract](docs/ARCHITECTURE.md).

## Compatibility and architectures

| Item | Requirement |
| :--- | :--- |
| Firmware | OpenWrt 23.05 / 24.10 (`opkg` and `.ipk`), and derivatives such as FriendlyWrt and ImmortalWrt; fw4 (nftables) firewall required |
| Dependencies | `nftables` (shipped with fw4 firmware), `kmod-nft-nat`, `ca-bundle`; the LuCI app needs `luci-base` (which provides `ucode`) |
| Clients | Must use the DNS server handed out by the router; DoH / DoT enabled on a device or browser bypasses forwarding |

Releases include core packages and static binaries for the architectures below. The LuCI app and Chinese language pack are `all` packages that work everywhere.

| `uname -m` | Typical devices | Release package arch | Static binary |
| :--- | :--- | :--- | :--- |
| `aarch64` | Rockchip, MediaTek Filogic, Amlogic and other ARM64 routers | `aarch64_generic` | `xboxdownloadd-aarch64-unknown-linux-musl` |
| `x86_64` | x86 soft routers and virtual machines | `x86_64` | `xboxdownloadd-x86_64-unknown-linux-musl` |
| `armv7l` | IPQ40xx and other ARMv7 hard-float routers | `arm_cortex-a7_neon-vfpv4` | `xboxdownloadd-armv7-unknown-linux-musleabihf` |
| `mips` (little-endian MT7621) | MT7621 routers | `mipsel_24kc` | `xboxdownloadd-mipsel-unknown-linux-musl` |

Run `opkg print-architecture` to see which package architectures your firmware accepts. If the release architecture is missing (for example an ARM64 device that lists `aarch64_cortex-a53`), the CPU instruction set is the same but the package label differs: use the [static binary deployment](#option-2-deploy-the-static-binary-manually) below, or build with the SDK for your target as described in the [development guide](docs/DEVELOPMENT.md). Big-endian MIPS (such as ath79) has no release package yet.

## Installation

### Option 1: install IPK packages (recommended)

1. From this repository's Releases page, download matching versions of:
   - the core package `xboxdownload_<version>_<package-arch>.ipk`
   - the LuCI app `luci-app-xboxdownload_<version>_all.ipk`
   - optionally the Chinese language pack `luci-i18n-xboxdownload-zh-cn_<version>_all.ipk`

   Verify the files against the release `SHA256SUMS`.
2. Upload them to `/tmp` on the router and run the following in the router's SSH session (replace `<package-arch>` with the value from the table above):

```sh
opkg update
opkg install /tmp/xboxdownload_*_<package-arch>.ipk /tmp/luci-app-xboxdownload_*_all.ipk
opkg install /tmp/luci-i18n-xboxdownload-zh-cn_*_all.ipk   # optional: Chinese UI
/etc/init.d/rpcd reload
```

### Option 2: deploy the static binary manually

Use this when the package architecture label does not match or you prefer not to use opkg. On your computer, download `xboxdownloadd-<target-triple>` for your architecture and check out the source at the **same tag** (it provides `data/` and the scripts under `openwrt/`). From the repository root:

```sh
router=root@192.168.1.1            # replace with your router address
bin=xboxdownloadd-aarch64-unknown-linux-musl   # replace with your architecture
ssh "$router" 'mkdir -p /tmp/xbd-install'
# Recent OpenSSH needs -O to talk to OpenWrt's Dropbear
scp -O -r "$bin" data openwrt/xboxdownload/files "$router:/tmp/xbd-install/"
```

Then install the core files on the router, keeping any existing configuration and adding the custom IP directory to upgrade backups:

```sh
cd /tmp/xbd-install
bin=$(ls xboxdownloadd-*)
opkg update && opkg install kmod-nft-nat ca-bundle
command -v nft >/dev/null || opkg install nftables-json
mkdir -p /usr/bin /usr/libexec/xboxdownload /usr/share/xboxdownload/ip /etc/hotplug.d/iface /etc/xboxdownload
cp "$bin" /usr/bin/xboxdownloadd
cp files/xboxdownload.init /etc/init.d/xboxdownload
cp files/xboxdownload-apply-ip /usr/libexec/xboxdownload/apply-ip
cp files/95-xboxdownload.hotplug /etc/hotplug.d/iface/95-xboxdownload
chmod 755 /usr/bin/xboxdownloadd /etc/init.d/xboxdownload /usr/libexec/xboxdownload/apply-ip
chmod 644 /etc/hotplug.d/iface/95-xboxdownload
cp data/IP.*.txt /usr/share/xboxdownload/ip/
[ -f /etc/config/xboxdownload ] || cp files/xboxdownload.config /etc/config/xboxdownload
grep -qxF /etc/xboxdownload/ /etc/sysupgrade.conf || echo /etc/xboxdownload/ >> /etc/sysupgrade.conf
/etc/init.d/xboxdownload enable
```

On a router that already has LuCI, copy the interface from the repository root on your computer (English UI; the Chinese language pack comes from the SDK build or the release IPK):

```sh
scp -O -r openwrt/luci-app-xboxdownload/htdocs/* "$router:/www/"
scp -O -r openwrt/luci-app-xboxdownload/root/usr/* "$router:/usr/"
ssh "$router" 'chmod 755 /usr/libexec/rpcd/luci.xboxdownload && rm -f /tmp/luci-indexcache.* && /etc/init.d/rpcd reload'
```

To build IPKs or cross-compile the binary yourself, see the [development guide](docs/DEVELOPMENT.md).

## First-time setup

1. Log in to LuCI again and open **Services → Xbox Download → Settings**.
2. Keep the default **Automatic dnsmasq forwarding** and **LAN alias IP** modes, check the LAN interface name (usually `br-lan`) and click **Suggest LAN address**. Make sure the address is outside the DHCP pool and not assigned to a sleeping device.
3. Leave **Fallback DNS server** empty to follow the dnsmasq upstream automatically. To pin a resolver, choose one from the list or enter `IPv4:port` or `[IPv6]:port`. Clear a saved address and apply to restore automatic selection.
4. Tick **Enable service**, then save and apply.
5. Open **Node speed test**, test the CN1, CN2 and Xbox App groups and click **Apply** on the fastest node (or wait for the scheduled test).
6. Back on **Overview**, click **Run end-to-end check**. In the default mode all 7 checks should pass.
7. Start a download on the Xbox or PC and confirm the client IP, Host, path and redirect target in the Overview log; entries created by the router self-check come from the router itself, so tell them apart from real downloads.

For typical hardware and more complex topologies (single-port soft routers, coexisting SmartDNS, main router + side router, multiple VLANs or dual-stack IPv6), see the [deployment cases](docs/CASES.md).

## Speed tests and daily use

**Node speed test** measures TCP latency first, then runs HTTP Range downloads against the shortlisted nodes. Speed is shown in MB/s and latency in ms. Results can be filtered by ISP, region or note, sorted by speed or latency and applied with one click. The `speed_mbps`, `last_speed_mbps` and `min_speed_mbps` fields are all in **MB/s (megabytes per second)**.

**Scheduled speed tests** are enabled by default: every 12 hours, preferably at 04:00 local time; deferred if a download redirect happened in the last 15 minutes; a healthy node is replaced only by one at least 20% faster; a failed node or one below 5 MB/s is replaced early; a fully failed run keeps the selected IP. Applying a node updates the running state, persists UCI and flushes the dnsmasq cache. Concurrency, candidate count, range size, timeouts and the IP list sync mirror (GitHub Raw, jsDelivr or a custom URL) are adjustable on the Settings page.

Useful commands:

```sh
ubus call luci.xboxdownload status
ubus call luci.xboxdownload self_check
ubus call luci.xboxdownload logs '{"limit":10}'
logread -e xboxdownload
```

## Quick Deployment for AI Agents

> This section is for AI coding assistants (Claude Code, Codex, OpenClaw and others) operating a router on the user's behalf. Run every command in the **router's SSH session** unless marked "on the computer". Each step has a checkable expected result; if a result does not match, stop and report to the user instead of guessing or forcing ahead.

### 0. Boundaries

- Modify only `/etc/config/xboxdownload` and the files installed by the packages. **Do not** modify `/etc/config/uhttpd`, `/etc/config/firewall`, `/etc/config/dhcp`, or SmartDNS / AdGuardHome configuration.
- The alias IP must be inside the LAN subnet, outside the DHCP pool and currently unused. Ask the user when unsure.
- The upstream DNS must not point at the router's own port 53 or at the daemon port (5353 by default); the init script refuses to start otherwise.

### 1. Preflight checks

```sh
cat /etc/openwrt_release                     # DISTRIB_RELEASE should be 23.05.x or 24.10.x
uname -m                                     # aarch64 / x86_64 / armv7l / mips
opkg print-architecture                      # accepted package architectures
command -v nft fw4 ucode jsonfilter          # all four should exist (ucode comes with LuCI)
ls /var/etc/dnsmasq.conf.*                   # at least one running dnsmasq instance
ubus call network.interface.lan status | jsonfilter -e '@.l3_device' -e '@["ipv4-address"][0].address'
uci -q get dhcp.lan.start; uci -q get dhcp.lan.limit   # DHCP pool range
df -h /overlay                               # more than 4 MB free
```

### 2. Architecture mapping

| `uname -m` | `opkg print-architecture` contains | Use |
| :--- | :--- | :--- |
| `aarch64` | `aarch64_generic` | `xboxdownload_*_aarch64_generic.ipk` |
| `aarch64` | another `aarch64_*` | manual deployment of `xboxdownloadd-aarch64-unknown-linux-musl` |
| `x86_64` | `x86_64` | `xboxdownload_*_x86_64.ipk` |
| `armv7l` | `arm_cortex-a7_neon-vfpv4` | `xboxdownload_*_arm_cortex-a7_neon-vfpv4.ipk` |
| `armv7l` | another hard-float `arm_*` | manual deployment of `xboxdownloadd-armv7-unknown-linux-musleabihf` |
| `mips` | `mipsel_24kc` | `xboxdownload_*_mipsel_24kc.ipk` |

The LuCI app and the Chinese pack are always the `_all.ipk` files. Installation steps are under [Installation](#installation).

### 3. Minimal UCI configuration

```sh
LAN_DEV=$(ubus call network.interface.lan status | jsonfilter -e '@.l3_device')
# With the LuCI app installed the router can suggest a free address; otherwise pick a free one outside the DHCP pool
ubus call luci.xboxdownload suggest_alias_ip "{\"lan_device\":\"$LAN_DEV\"}"

uci set xboxdownload.main.lan_device="$LAN_DEV"
uci set xboxdownload.main.alias_ip='192.168.1.253'      # replace with the confirmed free address
uci set xboxdownload.main.enabled='1'
uci commit xboxdownload
/etc/init.d/xboxdownload enable
/etc/init.d/xboxdownload restart
```

### 4. Select nodes

```sh
ubus call luci.xboxdownload speedtest_start '{"group":"xbox_cn1","auto_apply":true}'
ubus call luci.xboxdownload speedtest_status          # repeat until "running": false
# Repeat for xbox_cn2 and xbox_app; only one speed test can run at a time
ubus call luci.xboxdownload status                    # groups.*.selected_ipv4 should now be set
```

### 5. Acceptance

```sh
ubus call luci.xboxdownload self_check
```

Expect `"all_passed": true` with 7 checks in the default mode: `dns_redirect_a`, `dns_redirect_aaaa`, `dns_srv_block`, `http_redirect`, `http_dlassets_redirect`, `dnsmasq_redirect_a`, `http_alias_redirect`. Then run:

```sh
nft list table inet xboxdownload                       # DNAT rules exist
ip -4 addr show dev "$LAN_DEV" | grep "$(uci get xboxdownload.main.alias_ip)/32"
ls /tmp/dnsmasq*.d/xboxdownload.conf                   # fragment installed
nslookup xvcf1.xboxlive.com 127.0.0.1                  # answers with the alias IP
```

On a LAN client (optional, requires the user): `curl -sD - -o /dev/null http://xvcf1.xboxlive.com/Z/routing/extraextralarge.txt` should return `302` with `Location` pointing to `http://assets1.xboxlive.cn/...`.

### 6. Rollback

```sh
/etc/init.d/xboxdownload stop          # removes alias IP, nftables table and dnsmasq fragment
/etc/init.d/xboxdownload disable
uci set xboxdownload.main.enabled='0' && uci commit xboxdownload
# Full removal (optional)
opkg remove luci-i18n-xboxdownload-zh-cn luci-app-xboxdownload xboxdownload
# Verify the rollback
nft list table inet xboxdownload 2>&1 | grep -q 'No such file' && echo 'nft clean'
ls /tmp/dnsmasq*.d/xboxdownload.conf 2>/dev/null || echo 'dnsmasq clean'
```

### 7. Quick diagnosis

| Symptom | Check |
| :--- | :--- |
| Service does not start | `logread -e xboxdownload`: usually an empty or invalid `alias_ip`, a looping `upstream_dns` or a port conflict |
| `dnsmasq_redirect_a` fails | dnsmasq `conf-dir` was not found, or DNS rebind protection blocked the private answer; inspect `conf-dir=` in `/var/etc/dnsmasq.conf.*` |
| `http_alias_redirect` fails | A network reload removed the alias IP, or the nftables table is missing; run `/etc/init.d/xboxdownload restart` |
| Self-check passes but the console is not redirected | The client does not use the router DNS or has DoH enabled; check `logs` for entries from that client |

## Troubleshooting

If the self-check passes but real clients are not redirected, run `nslookup xvcf1.xboxlive.com <router address>` from a computer on the same LAN, then `curl -sD - -o /dev/null http://xvcf1.xboxlive.com/Z/routing/extraextralarge.txt` to confirm the 302 and its Location. Common causes:

- The console or PC uses custom DNS or DoH / DoT and bypasses the router's dnsmasq.
- A transparent proxy or side router upstream takes over DNS; see the [deployment cases](docs/CASES.md).
- In custom HTTP port mode, you must make sure TCP port 80 traffic to the target IPv4 address reaches the listener.

## Disable and uninstall

- Temporarily disable: untick **Enable service** in LuCI and apply, or run `/etc/init.d/xboxdownload stop`.
- Uninstall: `opkg remove luci-i18n-xboxdownload-zh-cn luci-app-xboxdownload xboxdownload`. Custom IP lists in `/etc/xboxdownload/` are kept; delete them manually when no longer needed.

## Privacy

This project has **zero telemetry**: speed-test results, node IPs, client information and redirect logs are never collected or reported, and logs stay in router memory. To function, the router makes the following outbound connections, all of which you control:

| Connection | Purpose | How to disable or replace |
| :--- | :--- | :--- |
| `skydevil88/XboxDownload` IP lists on GitHub Raw / jsDelivr | Scheduled sync of candidate node lists | Disable IP list sync on the Settings page, or use a custom mirror URL |
| `xbox.skydevil.xyz` package URL resolver | Fetches a real package URL for Xbox App speed tests | Set a custom test URL for that group; falls back to TCP RTT ranking when unreachable |
| Candidate CDN nodes | TCP latency and HTTP Range download tests | Disable scheduled speed tests |
| Fallback upstream DNS (`upstream_dns`) | Resolves records of managed hosts that are not rewritten | Defaults to the dnsmasq upstream, then WAN DNS, then `223.5.5.5:53`. Enter a trusted resolver to pin it |

## FAQ

<details>
<summary>Does it affect Windows Update or other Microsoft services?</summary>

Only exact Xbox and Microsoft Store download hostnames are taken over. A CDN group whose hostnames are shared with other services is taken over only after you select a node for it; until then those requests use your existing DNS as usual.

</details>

<details>
<summary>Does it take port 80 or break the LuCI login?</summary>

No. The default mode uses a dedicated LAN `/32` alias IP, and only port 80 traffic to that address is DNATed to the daemon. Port 80 on the router's own LAN IP is still served by uhttpd.

</details>

<details>
<summary>I already use SmartDNS / AdGuardHome / OpenClash. Will it still work?</summary>

Yes. Their configuration is never modified. When dnsmasq remains the DNS entry point for LAN clients, use the default automatic forwarding. If clients talk to AdGuardHome or another DNS directly, switch to manual mode and copy the generated rules. See the [deployment cases](docs/CASES.md) for specific topologies.

</details>

<details>
<summary>Is OpenWrt 25.x (apk package manager) supported?</summary>

Releases currently ship `.ipk` packages for opkg-based 23.05 and 24.10. On newer firmware you can try the manual static binary deployment.

</details>

## Documentation map

| Document | Audience | Content |
| :--- | :--- | :--- |
| [README.md](README.md) / [README_EN.md](README_EN.md) | End users and their AI agents | Installation, setup, usage, troubleshooting |
| [docs/CASES.md](docs/CASES.md) | End users | Anonymized hardware and network topology cases |
| [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) | Developers | Domain rules, UCI / JSON / ubus contracts |
| [docs/DEVELOPMENT.md](docs/DEVELOPMENT.md) | Developers | Build, cross-compilation, SDK packaging, testing and debugging |
| [CONTRIBUTING.md](CONTRIBUTING.md) | Contributors | Issue / PR guidelines and checklist |
| [AGENTS.md](AGENTS.md) | AI agents and developers extending the project | Repository layout, architecture invariants and verification commands |

<details>
<summary>Project info: suggested GitHub About and Topics</summary>

**About**: OpenWrt LuCI plugin that accelerates Xbox / Microsoft Store downloads with router-side CDN speed testing and precise dnsmasq DNS forwarding. Rust daemon, zero telemetry.

**Topics**: `openwrt` `luci` `luci-app` `xbox` `xbox-series-x` `xbox-one` `microsoft-store` `game-pass` `download-accelerator` `cdn` `speedtest` `dns` `dnsmasq` `smartdns` `adguardhome` `nftables` `rust` `immortalwrt` `friendlywrt` `router`

</details>

## Credits and license

- Domain rules, the speed-testing approach and IP databases come from [skydevil88/XboxDownload](https://github.com/skydevil88/XboxDownload). Many thanks to its authors and maintainers.
- This project is released under the [GNU General Public License v3.0 or later (GPL-3.0-or-later)](LICENSE).
- Xbox, Xbox Series X|S, Xbox One, Microsoft Store and PC Game Pass are trademarks of Microsoft. This project is not affiliated with Microsoft.
