# Development Guide

This guide covers building, cross-compiling, packaging, testing and debugging XboxDownloadOpenWrt. End-user installation lives in [README.md](../README.md) / [README_EN.md](../README_EN.md). The cross-component contract (domain rules, UCI schema, runtime JSON, control socket and ubus API) is defined in [ARCHITECTURE.md](ARCHITECTURE.md); read it before changing any interface.

## 1. Repository layout

```text
.
├── Cargo.toml, Cargo.lock          # Rust workspace (release profile: opt-level=z, LTO, strip, panic=abort)
├── daemon/                         # xboxdownloadd: DNS, HTTP 302, speed test, scheduler, control socket
│   ├── src/
│   │   ├── main.rs                 # CLI: run | print-dnsmasq | ctl
│   │   ├── config.rs               # runtime JSON schema and defaults
│   │   ├── rules.rs                # exact managed hostnames and redirect targets
│   │   ├── dns.rs                  # DNS listener and upstream forwarding
│   │   ├── http_redirect.rs        # HTTP 302 listener
│   │   ├── ipdb.rs                 # bundled / cached / custom IP lists and mirror sync
│   │   ├── speedtest.rs            # TCP pre-filter + HTTP Range download test
│   │   ├── scheduler.rs            # periodic tests, deferral and hysteresis
│   │   ├── selfcheck.rs            # 7 end-to-end checks
│   │   ├── control.rs              # Unix socket protocol
│   │   ├── state.rs, net.rs        # shared state and network helpers
│   └── tests/end_to_end.rs         # integration tests
├── data/IP.*.txt                   # bundled public CDN IP lists (from upstream XboxDownload)
├── openwrt/
│   ├── xboxdownload/               # core OpenWrt package
│   │   ├── Makefile
│   │   └── files/                  # init script, apply-ip hook, hotplug script, default UCI config
│   └── luci-app-xboxdownload/      # LuCI package
│       ├── htdocs/                 # LuCI JS views, shared RPC module and CSS
│       ├── po/                     # translation template and zh_Hans translation
│       └── root/usr/               # rpcd backend, ACL and menu
├── docs/                           # ARCHITECTURE.md, DEVELOPMENT.md, CASES.md
└── .github/workflows/              # ci.yml (checks) and release.yml (SDK packaging)
```

## 2. Prerequisites

- A Unix development machine (Linux or macOS) with stable Rust from [rustup](https://rustup.rs/).
- For cross-compilation without an SDK: [Zig](https://ziglang.org/) 0.13 and `cargo install --locked cargo-zigbuild --version 0.20.1` (the versions CI uses).
- For `.ipk` packaging: a Linux x86_64 host and the OpenWrt SDK that matches the target firmware.
- For non-Rust checks: Node.js, Python 3, `shellcheck`, GNU gettext (`msgfmt`) and optionally `actionlint`.

## 3. Rust build and tests

Run these from the repository root. They are the same checks CI runs.

```sh
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
```

Build a native binary and inspect the generated dnsmasq fragment for a sample config:

```sh
cargo build --locked --release
tmp=$(mktemp -d)
echo '{"http":{"mode":"alias_ip","bind":"192.168.1.253","redirect_ip":"192.168.1.253"}}' > "$tmp/test.json"
./target/release/xboxdownloadd print-dnsmasq --config "$tmp/test.json"
```

Every JSON field has a serde default, so partial configs are valid.

## 4. Cross-compiling static binaries

Release binaries are static musl executables. The supported targets are:

| OpenWrt package arch | Rust target | Toolchain notes |
| :--- | :--- | :--- |
| `aarch64_generic` | `aarch64-unknown-linux-musl` | stable |
| `x86_64` | `x86_64-unknown-linux-musl` | stable |
| `arm_cortex-a7_neon-vfpv4` | `armv7-unknown-linux-musleabihf` | stable |
| `mipsel_24kc` | `mipsel-unknown-linux-musl` | Tier 3: nightly + `rust-src`, `-Zbuild-std=std,panic_abort` |

### 4.1 With cargo-zigbuild (no SDK)

```sh
rustup target add aarch64-unknown-linux-musl
cargo zigbuild --release --locked --target aarch64-unknown-linux-musl
ls -l target/aarch64-unknown-linux-musl/release/xboxdownloadd
```

Replace the target for x86_64 or armv7. For MIPS:

```sh
rustup toolchain install nightly --component rust-src
cargo +nightly zigbuild --release --locked --target mipsel-unknown-linux-musl -Zbuild-std=std,panic_abort
```

Confirm the result has no dynamic loader or shared library dependency (use `llvm-readelf` on macOS):

```sh
readelf -lW target/<triple>/release/xboxdownloadd | grep INTERP     # expect no output
readelf -dW target/<triple>/release/xboxdownloadd | grep NEEDED     # expect no output
```

### 4.2 With the OpenWrt SDK toolchain

The release workflow links with the SDK's own GCC so the binary matches the target libc and CPU flags. See the "Build and smoke-test a static target executable" step in [release.yml](../.github/workflows/release.yml) for the exact environment (`CARGO_TARGET_<TRIPLE>_LINKER`, `CC_<triple>`, `AR_<triple>`, `RUSTFLAGS='-C target-feature=+crt-static -C link-self-contained=no'`).

## 5. Building `.ipk` packages with the OpenWrt SDK

Use a Linux x86_64 host and a freshly extracted SDK that matches your firmware release and target, for example `https://downloads.openwrt.org/releases/<version>/targets/<target>/<subtarget>/`. Set `SDK` to that directory and run from the repository root:

```sh
repo="$PWD"
SDK=/path/to/openwrt-sdk
rustup target add <rust-target>          # see the table above
cd "$SDK"
./scripts/feeds update base packages luci
./scripts/feeds install ca-bundle nftables-json kmod-nft-nat luci-base
ln -s "$repo/openwrt/xboxdownload" package/xboxdownload
ln -s "$repo/openwrt/luci-app-xboxdownload" package/luci-app-xboxdownload
cat >> .config <<'EOF'
CONFIG_PACKAGE_xboxdownload=m
CONFIG_PACKAGE_luci-app-xboxdownload=m
CONFIG_LUCI_LANG_zh_Hans=y
CONFIG_PACKAGE_luci-i18n-xboxdownload-zh-cn=m
EOF
make defconfig
make package/xboxdownload/compile package/luci-app-xboxdownload/compile V=s
find bin/packages -name '*xboxdownload*.ipk'
```

Notes:

- The core package Makefile maps the SDK `ARCH` to a Rust target (`aarch64`, `x86_64`, `arm`, `mipsel`). For other architectures pass `XBOXDOWNLOAD_RUST_TARGET=<triple>` to `make`.
- To reuse an already built static binary, pass `XBOXDOWNLOAD_PREBUILT_BIN=/absolute/path/xboxdownloadd` to `make`; Cargo is then skipped.
- The core package must not depend on `flock`. Both the init script and the apply hook poll `flock -n`, which BusyBox provides. See [ARCHITECTURE.md §5](ARCHITECTURE.md#5-runtime-json-config-contract-varetcxboxdownloadjson).
- The release workflow builds the LuCI package once (in the aarch64 job) because it is architecture independent.

## 6. LuCI, rpcd, shell, translation and workflow checks

```sh
# LuCI JavaScript syntax
find openwrt -type f -name '*.js' -exec node --check {} \;

# JSON (menu, ACL)
find openwrt -type f -name '*.json' -exec python3 -m json.tool {} \; >/dev/null

# POSIX shell: init script, apply hook, hotplug script, rpcd backend
scripts="openwrt/xboxdownload/files/xboxdownload.init
openwrt/xboxdownload/files/95-xboxdownload.hotplug
openwrt/xboxdownload/files/xboxdownload-apply-ip
openwrt/luci-app-xboxdownload/root/usr/libexec/rpcd/luci.xboxdownload"
for f in $scripts; do sh -n "$f"; done
shellcheck -s sh -e SC1091,SC3043,SC2154 $scripts

# Translations: syntax, and the translation matches the template
msgfmt --check -o /dev/null openwrt/luci-app-xboxdownload/po/zh_Hans/xboxdownload.po
msgcmp openwrt/luci-app-xboxdownload/po/zh_Hans/xboxdownload.po openwrt/luci-app-xboxdownload/po/templates/xboxdownload.pot

# GitHub Actions workflows
ruby -ryaml -e 'ARGV.each { |f| YAML.load_file(f) }' .github/workflows/*.yml   # or python3 + PyYAML
actionlint                                                                      # if installed
```

When adding a user-visible LuCI string, wrap it in `_()`, add it to `po/templates/xboxdownload.pot` and translate it in `po/zh_Hans/xboxdownload.po`. LuCI views must build DOM nodes with `E()`; do not use `innerHTML`.

## 7. Testing on an isolated OpenWrt VM

Never test unreleased builds on a production router first. A QEMU virtual machine is enough for end-to-end checks:

1. Download an official OpenWrt image for a VM target (for example `x86/64` `generic-ext4-combined.img.gz`, or `armsr/armv8` for `qemu-system-aarch64`) of the release you want to test.
2. Boot it with user-mode networking and an SSH port forward, for example with `qemu-system-x86_64 -nographic -m 256 -drive file=openwrt.img,format=raw -nic user,model=virtio,hostfwd=tcp::2222-:22`. Inside the VM the LAN bridge is `br-lan` at `192.168.1.1/24`.
3. Copy the packages built in §5 (or the static binary and files, following the manual steps in the README) with `scp -O -P 2222`, install them and configure an alias IP such as `192.168.1.253`.
4. Run `ubus call luci.xboxdownload self_check` and expect all 7 checks to pass. Then exercise `restart`, `stop` (verify the alias IP, `table inet xboxdownload` and `xboxdownload.conf` are gone), `ifup lan` (verify the hotplug script restores the alias) and an `apply_ip` call.
5. Repeat on both 23.05 and 24.10 when touching dnsmasq integration, because their `conf-dir` layouts differ.

Speed tests need outbound internet access from the VM; DNS, HTTP 302 and self-check work offline.

## 8. Debugging

On the router:

```sh
logread -e xboxdownload -f                         # follow daemon and init logs
cat /var/etc/xboxdownload.json                     # rendered runtime config
cat /tmp/dnsmasq*.d/xboxdownload.conf              # installed dnsmasq fragment
nft list table inet xboxdownload                   # DNAT and optional IPv6 block rules
xboxdownloadd ctl --socket /var/run/xboxdownload.sock --method status
xboxdownloadd ctl --socket /var/run/xboxdownload.sock --method logs --params '{"limit":20}'
ubus -v list luci.xboxdownload                     # rpcd method signatures
```

On a development machine, run the daemon in the foreground with unprivileged ports and a private runtime directory:

```sh
tmp=$(mktemp -d)
cat > "$tmp/dev.json" <<EOF
{
  "dns": {"mode": "manual", "bind": "127.0.0.1", "port": 15353},
  "http": {"mode": "custom_port", "bind": "127.0.0.1", "port": 18080, "redirect_ip": "127.0.0.1"},
  "runtime": {"socket_path": "$tmp/xbd.sock", "data_dir": "$PWD/data", "cache_dir": "$tmp/cache",
              "custom_dir": "$tmp/custom", "apply_hook": ""},
  "speedtest": {"auto_enabled": false, "ip_sync_enabled": false}
}
EOF
cargo run --locked -- run --config "$tmp/dev.json"
# in another terminal
dig @127.0.0.1 -p 15353 xvcf1.xboxlive.com A
curl -sD - -o /dev/null -H 'Host: xvcf1.xboxlive.com' http://127.0.0.1:18080/Z/routing/extraextralarge.txt
cargo run --locked -- ctl --socket "$tmp/xbd.sock" --method self_check
```

With `dns.mode = "manual"` and `http.mode = "custom_port"` the self-check runs the 5 local checks; the `dnsmasq_redirect_a` and `http_alias_redirect` checks need a real router. Scheduled tests and IP sync are disabled here so the dev run makes no outbound connections.

## 9. Release process

1. Bump `PKG_VERSION` in `openwrt/xboxdownload/Makefile` and `version` in `daemon/Cargo.toml` together, then run `cargo check` once (without `--locked`) so `Cargo.lock` records the new version.
2. Push a `v<version>` tag. [release.yml](../.github/workflows/release.yml) verifies the tag matches `PKG_VERSION`, downloads and checksum-verifies the OpenWrt SDKs, builds and smoke-tests a static binary per architecture under QEMU user emulation, packages the `.ipk` files and publishes them with `SHA256SUMS`.
3. A failed release can be rebuilt for an existing tag through `workflow_dispatch`.

## 10. Further reading

- [ARCHITECTURE.md](ARCHITECTURE.md): design principles and every interface contract.
- [CASES.md](CASES.md): deployment topologies that new features must keep working.
- [CONTRIBUTING.md](../CONTRIBUTING.md): pull request rules and the pre-submit checklist.
- [AGENTS.md](../AGENTS.md): condensed invariants for AI coding agents.
