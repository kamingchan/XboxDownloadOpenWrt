# XboxDownloadOpenWrt

[简体中文](README.md) · [Architecture and API](docs/ARCHITECTURE.md) · [GPL-3.0-or-later](LICENSE)

Select CDN nodes for Xbox and Windows Microsoft Store downloads on an OpenWrt router. Clients use the router for DNS. LuCI shows recent redirects, measures candidate throughput and can switch nodes automatically during quiet periods. Thanks to [skydevil88/XboxDownload](https://github.com/skydevil88/XboxDownload) for the domain rules, speed-testing approach and IP databases.

## Request flow

```text
Xbox / PC ──DNS──► dnsmasq ──ordinary domains──► existing SmartDNS / upstream
                      └──download and SRV names──► xboxdownloadd :5353

Xbox / PC ──HTTP:80──► LAN alias IP ──dedicated DNAT──► HTTP :18080
          ◄──302 Location: http://assets1.xboxlive.cn/original-path──┘
          └──resolve .cn hostname──► selected CDN IP ──► direct download
```

The HTTP redirect preserves the path and query string. The client downloads the game directly from the CDN. Legacy dlassets requests go to dlassets.xboxlive.cn. Xbox App has its own download domains. Candidate lists cover CN1, CN2, Xbox App and Akamai. The Akamai group is disabled by default. Results depend on the ISP, local routing and time of day.

- Automatic mode discovers the active dnsmasq `conf-dir`, typically `/tmp/dnsmasq.d` on OpenWrt 23.05 or `/tmp/dnsmasq.<cfg>.d` on 24.10. It restarts dnsmasq once after adding, changing or removing `xboxdownload.conf`. Routine node switches within an active group send `SIGHUP` to flush the cache.
- Each enabled CDN group delegates its listed hostnames once it has a compatible IP. CN1, CN2 and Xbox App can each opt into an address from the enabled Akamai group. Groups without a usable node leave Windows Update and other household traffic on the existing SmartDNS or OpenClash resolver chain. Manual mode generates SmartDNS, AdGuard Home and dnsmasq rules. Copy and apply the updated rules when groups first acquire addresses or the set of delegated hostnames changes.
- Alias mode scopes DNAT to one LAN address in `table inet xboxdownload`. The uhttpd listener, fw4 main table and SmartDNS configuration need no changes. LAN `ifup` and `ifupdate` hotplug events restore the `/32` alias while the service is active. Stopping the service removes the alias, its own firewall rules and the temporary DNS snippet.
- LAN IPv6 stays enabled. Redirect domains return the alias for A and empty NOERROR for AAAA, HTTPS and SVCB. The designated SRV fallback records also receive empty answers. Each CDN group can use IPv4, IPv6 or dual stack. IPv4-only groups suppress records that could bypass the chosen node. IPv6-enabled groups can serve a selected IPv6 address.
- The optional per-MAC fallback blocks IPv6 forwarding and restricts DHCPv6 and ICMPv6 to the router for selected clients. Leave it empty unless a client repeatedly uses cached IPv6 routes.
- There is no telemetry. The service never reports speed-test results, node IP addresses or redirect logs to third parties. Unless you supply a custom test URL, Xbox App tests request a package URL from the upstream resolver and fall back to TCP RTT ranking if it is unreachable.
- Custom lists live in `/etc/xboxdownload/custom.<group>.txt`. They survive reboots and the IPK registers them for sysupgrade configuration backups. Synced IP lists are cached in `/var/lib/xboxdownload/`. Bundled addresses remain available after reboot and scheduled sync refreshes the cache when enabled.

## Install on a NanoPi R5C

The example targets OpenWrt **23.05.5**, **rockchip/armv8**. Check `ubus call system board` and `opkg print-architecture` before choosing a package. The official target uses `aarch64_generic`. Clients need to use the router DNS. Device-level DoH bypasses these forwarding rules.

### Install IPK packages

1. From this repository’s Releases page, choose matching versions of `xboxdownload_*_aarch64_generic.ipk`, `luci-app-xboxdownload_*_all.ipk` and, optionally, `luci-i18n-xboxdownload-zh-cn_*_all.ipk`. You can also build them with the SDK below. Verify each downloaded file against its entry in the release SHA256SUMS.
2. Upload the selected files to /tmp and run the following commands on the router. Install the translation package only if desired.

```sh
opkg update
opkg install /tmp/xboxdownload_*_aarch64_generic.ipk /tmp/luci-app-xboxdownload_*_all.ipk
opkg install /tmp/luci-i18n-xboxdownload-zh-cn_*_all.ipk
/etc/init.d/rpcd reload
```

3. Sign in to LuCI again and open **Services → Xbox Download → Settings**. Keep automatic dnsmasq forwarding and LAN alias mode. Check the bridge name and click **Suggest LAN address**. Reserve the address outside the DHCP pool and check that no sleeping device owns it. Select **Enable service**, then save and apply. Use an independent fallback DNS server to avoid a resolver loop.
4. Run the end-to-end check on **Overview**. The default dnsmasq and LAN alias configuration runs seven checks. In addition to three local DNS and two local HTTP checks, it queries dnsmasq on port 53 and tests the 302 through DNAT on alias port 80. These last two checks appear when their respective modes are active. Start a real download from an Xbox or PC, then check the client IP, host, path and target in the redirect log. The client IP distinguishes console traffic from the router’s self-checks.

### Deploy a standalone executable

Download `xboxdownloadd-aarch64-unknown-linux-musl` and place it at the root of the source checkout for the same tag. Run these commands on your computer, substituting the router address. Modern OpenSSH scp can use -O with Dropbear.

```sh
router=root@192.168.1.1
ssh "$router" 'mkdir -p /tmp/xbd-install'
scp -O -r xboxdownloadd-aarch64-unknown-linux-musl data openwrt/xboxdownload/files "$router:/tmp/xbd-install/"
```

Use the [core package install manifest](openwrt/xboxdownload/Makefile) for the same tag to locate the hotplug script installed as `/etc/hotplug.d/iface/95-xboxdownload`. Upload that script as `/tmp/xbd-install/95-xboxdownload`. On the router, install the service files and register the custom IP directory for upgrade backups. The configuration copy preserves an existing installation.

```sh
cd /tmp/xbd-install
opkg update && opkg install nftables-json ca-bundle flock
mkdir -p /usr/bin /usr/libexec/xboxdownload /usr/share/xboxdownload/ip /etc/hotplug.d/iface /etc/xboxdownload
cp xboxdownloadd-aarch64-unknown-linux-musl /usr/bin/xboxdownloadd
cp files/xboxdownload.init /etc/init.d/xboxdownload
cp files/xboxdownload-apply-ip /usr/libexec/xboxdownload/apply-ip
cp 95-xboxdownload /etc/hotplug.d/iface/95-xboxdownload
chmod 755 /usr/bin/xboxdownloadd /etc/init.d/xboxdownload /usr/libexec/xboxdownload/apply-ip /etc/hotplug.d/iface/95-xboxdownload
cp data/IP.*.txt /usr/share/xboxdownload/ip/
[ -f /etc/config/xboxdownload ] || cp files/xboxdownload.config /etc/config/xboxdownload
grep -qxF /etc/xboxdownload/ /etc/sysupgrade.conf || echo /etc/xboxdownload/ >> /etc/sysupgrade.conf
/etc/init.d/xboxdownload enable
```

If LuCI is already installed, copy the English interface from your computer’s source checkout. The SDK can also build its Simplified Chinese translation package. Use the same first-run steps as for an IPK installation.

```sh
scp -O -r openwrt/luci-app-xboxdownload/htdocs/* "$router:/www/"
scp -O -r openwrt/luci-app-xboxdownload/root/usr/* "$router:/usr/"
ssh "$router" 'chmod 755 /usr/libexec/rpcd/luci.xboxdownload && rm -f /tmp/luci-indexcache.* && /etc/init.d/rpcd reload'
```

For CLI-only operation, set the actual `lan_device`, a reserved `alias_ip` and `main.enabled` to 1 in `/etc/config/xboxdownload`, then run `/etc/init.d/xboxdownload start`.

## Node selection and verification

**Node speed test** measures TCP latency, then issues HTTP Range requests to shortlisted nodes. The UI reports throughput in MB/s and latency in ms. Filter by carrier, region or note, sort by speed or latency, and apply a result to its group. The fields `speed_mbps`, `last_speed_mbps` and `min_speed_mbps` all use MB/s (megabytes per second).

**Automatic tests** default to a 12-hour interval with a preferred local hour of 04:00. A recent HTTP 302 hit defers testing, with a default quiet window of 15 minutes. Replacing a healthy node normally requires a 20% improvement. A failed node or one below 5 MB/s can be replaced without that threshold. An entirely failed test retains the selected IP. Applying an address updates runtime routing, persists it to UCI and flushes dnsmasq’s cache. Concurrency, shortlist size, range length, timeouts and the IP sync mirror (GitHub Raw, jsDelivr or a custom base URL) are configurable.

```sh
ubus call luci.xboxdownload status
ubus call luci.xboxdownload self_check
ubus call luci.xboxdownload logs '{"limit":10}'
logread -e xboxdownload
```

If the router self-check succeeds but the client shows no hits, run `nslookup xvcf1.xboxlive.com <router-address>` from the same LAN, followed by `curl -sD - -o /dev/null http://xvcf1.xboxlive.com/Z/routing/extraextralarge.txt`. Verify the A record and the 302 Location. With a custom listener port, provide your own path from TCP port 80 on the advertised IPv4 address to the listener. Disable the service in LuCI and apply, or run `/etc/init.d/xboxdownload stop` for a temporary stop.

## Develop and cross-compile

The Rust workspace lives in daemon/ and the OpenWrt packages live in openwrt/. With stable Rust on a Unix development host, run `cargo test --workspace --locked`. The [architecture document](docs/ARCHITECTURE.md) defines the hostnames, UCI schema and RPC protocol.

On a Linux x86_64 build host use the [OpenWrt 23.05.5 rockchip/armv8 SDK](https://downloads.openwrt.org/releases/23.05.5/targets/rockchip/armv8/) matching the target firmware. Start at the repository root and set SDK to a freshly extracted SDK directory.

```sh
repo="$PWD"
SDK=/path/to/openwrt-sdk
rustup target add aarch64-unknown-linux-musl
cd "$SDK"
./scripts/feeds update base packages luci
./scripts/feeds install ca-bundle nftables-json flock luci-base
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

The core package uses the SDK target compiler and linker. To reuse a static target executable, pass `XBOXDOWNLOAD_PREBUILT_BIN=/absolute/path/to/xboxdownloadd` to make. See the [release workflow](.github/workflows/release.yml) for other target triples. Licensed under the [GNU GPL, version 3 or later](LICENSE).
