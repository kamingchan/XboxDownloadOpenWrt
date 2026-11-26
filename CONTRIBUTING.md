# Contributing to XboxDownloadOpenWrt

Thanks for helping improve XboxDownloadOpenWrt. Issues and pull requests are welcome in Chinese or English. 欢迎使用中文或英文提交 Issue 和 PR。

Before you start, read:

- [README.md](README.md) / [README_EN.md](README_EN.md) for what the project does and how users deploy it.
- [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for the contract shared by the Rust daemon, OpenWrt package and LuCI app.
- [docs/DEVELOPMENT.md](docs/DEVELOPMENT.md) for build, test and debugging recipes.
- [AGENTS.md](AGENTS.md) if you work with an AI coding agent; it condenses the invariants and checks below.

## Reporting issues

Search existing issues first. A useful bug report includes:

- Firmware and version: `cat /etc/openwrt_release` (OpenWrt, FriendlyWrt, ImmortalWrt …).
- Architecture: `uname -m` and `opkg print-architecture`.
- Package version and install method (IPK or manual binary).
- DNS topology: what clients use for DNS, and whether SmartDNS, AdGuardHome, OpenClash or a side router is involved. [docs/CASES.md](docs/CASES.md) has the common shapes.
- Output of `ubus call luci.xboxdownload self_check` and relevant lines from `logread -e xboxdownload`.
- What you expected and what happened.

**Redact before posting**: replace your public IP, LAN addresses, hostnames, MAC addresses, usernames, PPPoE or Wi-Fi credentials and any tokens with placeholders such as `192.168.1.1`, `aa:bb:cc:dd:ee:ff` or `<redacted>`. Never paste full configuration dumps that contain secrets.

Feature requests should describe the problem and the network topology it applies to. Security issues: report them privately through GitHub's private vulnerability reporting (**Security → Report a vulnerability**) rather than in a public issue.

## Pull requests

### Scope

- One topic per PR. Keep refactors separate from behaviour changes.
- Stay inside the module the change belongs to (`daemon/`, `openwrt/xboxdownload/`, `openwrt/luci-app-xboxdownload/`, `docs/`, `.github/workflows/`).
- A change to any interface (UCI option, runtime JSON field, control-socket method, ubus method or ACL) updates every layer in the same PR: Rust, UCI default and init renderer, rpcd backend and ACL, LuCI caller and translations, and `docs/ARCHITECTURE.md`.
- `data/IP.*.txt` mirrors upstream [XboxDownload](https://github.com/skydevil88/XboxDownload); update it only by syncing from upstream, in a dedicated PR.
- Changes must preserve the architecture invariants listed in [AGENTS.md](AGENTS.md#architecture-invariants), in particular: no edits to uhttpd, fw4 or other services' configuration; exact hostnames only; fixed runtime paths; DNS loop protection; BusyBox-compatible locking; zero telemetry.

### Documentation

| Change | Update |
| :--- | :--- |
| User-visible behaviour or installation | `README.md` and `README_EN.md` together |
| Build, test or release process | `docs/DEVELOPMENT.md` |
| Interfaces and contracts | `docs/ARCHITECTURE.md` |
| New supported topology or device experience | `docs/CASES.md` (anonymized) |

READMEs stay hardware-neutral; device-specific notes go to `docs/CASES.md`.

### Required checks

Run the checks for every area you touched. CI runs the Rust, cross-build and OpenWrt script checks on every push and pull request.

```sh
# Rust
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked

# Shell scripts (init, apply-ip hook, hotplug, rpcd backend)
sh -n <script>
shellcheck -s sh -e SC1091,SC3043,SC2154 <script>

# LuCI JavaScript and JSON
find openwrt -type f -name '*.js' -exec node --check {} \;
find openwrt -type f -name '*.json' -exec python3 -m json.tool {} \; >/dev/null

# Translations
msgfmt --check -o /dev/null openwrt/luci-app-xboxdownload/po/zh_Hans/xboxdownload.po
msgcmp openwrt/luci-app-xboxdownload/po/zh_Hans/xboxdownload.po openwrt/luci-app-xboxdownload/po/templates/xboxdownload.pot

# Workflows (if changed)
ruby -ryaml -e 'ARGV.each { |f| YAML.load_file(f) }' .github/workflows/*.yml
actionlint   # if installed

# Whitespace
git diff --check
```

For changes that affect router behaviour (init script, hotplug, rpcd, DNS or HTTP handling), also test on an isolated OpenWrt VM or spare router as described in [docs/DEVELOPMENT.md](docs/DEVELOPMENT.md#7-testing-on-an-isolated-openwrt-vm), and state in the PR which OpenWrt versions you tested and that `self_check` passed.

### Privacy and anonymization checklist

Before opening the PR, confirm that the diff contains none of the following:

- [ ] Absolute paths from your machine (for example under `/Users/` or `/home/`).
- [ ] Your hostnames, usernames, router names or device serials.
- [ ] Real public IP addresses or your own LAN layout. Use `192.168.1.0/24`, `10.0.0.0/24`, `192.0.2.0/24`, `198.51.100.0/24` or `2001:db8::/32` in examples. Public CDN IPs in `data/` and existing tests are project data and are fine.
- [ ] MAC addresses other than placeholders such as `aa:bb:cc:dd:ee:ff`.
- [ ] Tokens, passwords, keys or cookies.
- [ ] Logs, router dumps, packet captures, build artifacts (`*.ipk`, `target/`) or scratch files.

A quick scan:

```sh
git diff --cached | grep -nE '/Users/|/home/|BEGIN [A-Z ]*PRIVATE KEY|(token|password|secret)[[:space:]]*[:=]'
```

### Commits and review

- Write commit messages in the imperative mood with a conventional prefix, for example `feat: detect upstream DNS servers in LuCI settings` or `fix: support BusyBox flock for OpenWrt service locks`.
- Explain in the PR description what changed, why, and how you verified it.
- Keep the PR rebased on `main`; maintainers squash or merge after review and green CI.

## License

By contributing, you agree that your contributions are licensed under the [GNU General Public License v3.0 or later](LICENSE), the same license as the project.
