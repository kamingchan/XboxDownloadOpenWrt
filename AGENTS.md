# AGENTS.md

Guide for AI coding agents and developers extending XboxDownloadOpenWrt: an OpenWrt package pair (Rust daemon `xboxdownloadd` + LuCI app) that forwards exact Xbox / Microsoft Store download hostnames through dnsmasq, answers HTTP with a 302 to `.cn` hosts, and speed-tests CDN nodes.

## Read before editing

- [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md): the **contract**. Read before changing any domain rule, UCI option, runtime JSON field, control-socket method or ubus method.
- [docs/DEVELOPMENT.md](docs/DEVELOPMENT.md): build, cross-compile, SDK packaging, VM testing and debugging recipes.
- [CONTRIBUTING.md](CONTRIBUTING.md): PR scope, checklist and the privacy review.

## Layout and write scope

| Path | Owns | Language |
| :--- | :--- | :--- |
| `daemon/` | DNS, HTTP 302, speed test, scheduler, self-check, Unix control socket, `run` / `print-dnsmasq` / `ctl` CLI | Rust 2024, tokio, rustls |
| `openwrt/xboxdownload/` | Core package: `Makefile`, `files/` (procd init, apply-ip hook, iface hotplug, default UCI) | OpenWrt make, POSIX `sh` |
| `openwrt/luci-app-xboxdownload/` | LuCI JS views, shared RPC module, CSS, rpcd backend, ACL, menu, `po/` | LuCI JS, `sh` + `ucode`, gettext |
| `data/` | Bundled public CDN IP lists mirrored from upstream XboxDownload | text; refresh from upstream only |
| `docs/` | Contract, developer guide, user deployment cases | Markdown |
| `.github/workflows/` | `ci.yml` checks, `release.yml` SDK packaging | GitHub Actions |

Keep each change inside the module the task names. A cross-module change is one change: edit every layer it touches in the same commit.

## Architecture invariants

1. **Four-layer contract.** A field or method exists in all layers or none: Rust (`config.rs` / `control.rs`) ↔ UCI default (`files/xboxdownload.config`) and init renderer (`xboxdownload.init`) ↔ rpcd (`list` JSON + dispatcher) ↔ ACL + LuCI caller (`xboxdownload.js`, views, `po/`) ↔ `docs/ARCHITECTURE.md`. The rpcd object currently has 13 methods.
2. **Zero blast radius.** Router integration lives only in `table inet xboxdownload`, one `/32` alias IP and one `xboxdownload.conf` dnsmasq fragment, all removed on stop. uhttpd, `table inet fw4`, `/etc/config/firewall`, `/etc/config/dhcp`, SmartDNS and AdGuardHome configuration stay untouched.
3. **Exact hostnames.** Forward only the listed redirect, SRV and CDN hostnames. A CDN group is delegated only once it has a selected IP or an active Akamai fallback, so unconfigured groups leave Windows Update on the household resolver chain.
4. **Fixed runtime paths.** Socket, data, cache, custom and hook paths are hard-coded in `xboxdownload.init` and rendered into root-owned `/var/etc/xboxdownload.json`; UCI and LuCI never supply them.
5. **Loop guard on every interface.** `upstream_dns` is rejected when it resolves to the daemon listener or to any router address on port 53 (IPv4-mapped IPv6 included). The init check and `suggest_upstream_dns` enforce the same rule.
6. **BusyBox-portable locking.** Lock by polling `flock -n <fd>` with a bounded retry loop; the core package carries no `flock` dependency.
7. **Version-portable dnsmasq.** Discover `conf-dir` from every `/var/etc/dnsmasq.conf.*` (23.05 `/tmp/dnsmasq.d`, 24.10 `/tmp/dnsmasq.<cfg>.d`). Restart dnsmasq only when the fragment is added, changed or removed; IP switches send `SIGHUP`.
8. **Safe LuCI DOM.** Build nodes with `E()` and text content; every user-visible string goes through `_()` and both `po/` files.
9. **Zero telemetry.** Outbound traffic is limited to IP list sync mirrors, the Xbox App package resolver (skipped with a custom test URL), CDN speed tests and the configured upstream DNS. Any new outbound connection needs an opt-out and a line in the README privacy tables.
10. **Units.** `speed_mbps`, `last_speed_mbps` and `min_speed_mbps` are MB/s (megabytes per second).

## Verification by module

Run the rows for every module you touched; all must pass before you report done.

| Touched | Commands |
| :--- | :--- |
| `daemon/`, `Cargo.*` | `cargo fmt --check` · `cargo clippy --locked --all-targets -- -D warnings` · `cargo test --locked` |
| Cross-target behaviour | `cargo zigbuild --release --locked --target aarch64-unknown-linux-musl` (see DEVELOPMENT §4) |
| `openwrt/**/*.sh`, init, hotplug, rpcd | `sh -n <file>` · `shellcheck -s sh -e SC1091,SC3043,SC2154 <file>` |
| LuCI JS / JSON | `node --check <file.js>` · `python3 -m json.tool <file.json>` |
| `po/` | `msgfmt --check -o /dev/null po/zh_Hans/xboxdownload.po` · `msgcmp po/zh_Hans/xboxdownload.po po/templates/xboxdownload.pot` |
| `.github/workflows/` | YAML parse (`ruby -ryaml` or PyYAML) · `actionlint` when installed |
| Router-facing behaviour | Isolated OpenWrt VM: install, `ubus call luci.xboxdownload self_check` → 7/7, then `stop` leaves no alias, table or fragment (DEVELOPMENT §7) |
| Any Markdown | Every relative link resolves to an existing file · `git diff --check` |

## Documentation ownership

| Content | Home |
| :--- | :--- |
| Install, setup, usage, troubleshooting, privacy, AI-agent deployment steps | `README.md` (Chinese) and `README_EN.md` (English), kept in sync |
| Build, cross-compile, SDK, tests, debugging | `docs/DEVELOPMENT.md` |
| Domain rules, UCI / JSON / socket / ubus contracts | `docs/ARCHITECTURE.md` |
| Hardware and topology examples | `docs/CASES.md` |
| PR rules and checklists | `CONTRIBUTING.md` |

Keep the READMEs hardware-neutral; device models belong in `docs/CASES.md`.

## Privacy rules

Every committed file uses generic placeholders: RFC 1918 / RFC 5737 / RFC 3849 addresses (`192.168.1.1/24`, `10.0.0.1/24`, `192.0.2.0/24`, `2001:db8::/32`), `127.0.0.1:6053`, `aa:bb:cc:dd:ee:ff`, `/path/to/...`. Public CDN IPs in `data/IP.*.txt` and the tests are project data. Before committing, scan the diff for absolute home paths, personal hostnames or usernames, real public or LAN addresses from your own network, logs, router dumps and credentials, and keep scratch files out of the tree.
