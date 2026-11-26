# XboxDownloadOpenWrt

[English](README_EN.md) · [架构与接口](docs/ARCHITECTURE.md) · [GPL-3.0-or-later](LICENSE)

在 OpenWrt 路由器上为 Xbox 和 Windows Microsoft Store 下载选择 CDN 节点。客户端使用路由器 DNS 即可。在 LuCI 中可以查看重定向记录、实测候选节点，并让路由器在空闲时自动切换到更快的节点。感谢 [skydevil88/XboxDownload](https://github.com/skydevil88/XboxDownload) 提供的域名规则、测速思路和 IP 数据库。

## 请求如何流动

```text
Xbox / PC ──DNS──► dnsmasq ──普通域名──► 原有 SmartDNS / 上游
                      └──下载及 SRV 域名──► xboxdownloadd :5353

Xbox / PC ──HTTP:80──► LAN 辅助 IP ──独立 DNAT──► HTTP :18080
          ◄──302 Location: http://assets1.xboxlive.cn/原路径──┘
          └──解析 .cn 域名──► 选定的 CDN IP ──► 直接下载
```

HTTP 服务保留下载路径和查询参数，游戏文件由 CDN 直接传给客户端。旧版 dlassets 请求转向 dlassets.xboxlive.cn。Xbox App 使用自己的下载域名。节点分为 CN1、CN2、Xbox App、Akamai 四组，Akamai 组默认关闭。实际收益取决于线路、运营商和时段。

- 自动模式探测运行中的 dnsmasq 实例的 `conf-dir`，在 OpenWrt 23.05 通常为 `/tmp/dnsmasq.d`，在 24.10 通常为 `/tmp/dnsmasq.<cfg>.d`。只有新增、变更或删除 `xboxdownload.conf` 才会在片段更新完毕后重启一次 dnsmasq。日常切换同一分组的节点只发送 `SIGHUP` 清除 DNS 缓存。
- CDN 分组启用且选定兼容的 IP 后才接管该组列出的主机名。CN1、CN2 和 Xbox App 可以分别启用 Akamai 备用节点。分组没有可用节点时，Windows Update 等请求仍走家中原有的 SmartDNS 或 OpenClash 解析链。手动模式可复制 SmartDNS、AdGuard Home 或 dnsmasq 规则；首次选定节点或更改参与分流的组后，需重新复制并应用规则。
- 辅助 IP 模式只在独立的 `table inet xboxdownload` 中配置 DNAT，uhttpd 的监听端口、fw4 主表和 SmartDNS 配置均无需调整。服务运行期间，LAN 接口发生 `ifup` 或 `ifupdate` 时，热插拔脚本会恢复 `/32` 辅助 IP。停止服务会清理该地址、自有规则和临时 DNS 片段。
- LAN 保持 IPv6。重定向域的 A 指向辅助 IP，AAAA、HTTPS、SVCB 返回空的 NOERROR，指定的 SRV 回退记录也返回空响应。CDN 组可独立选 IPv4、IPv6 或双栈；IPv4 组抑制绕过节点选择的记录，IPv6 组可返回选定的 IPv6 节点。
- 可选 MAC 策略会阻止指定设备的 IPv6 转发，并限制它们发往路由器的 DHCPv6 和 ICMPv6。仅在个别设备反复使用缓存的 IPv6 路径时填写。
- 零遥测。服务不会向第三方上报测速结果、节点 IP 或命中日志。Xbox App 未填写自定义测速 URL 时会请求上游解析接口获取包下载地址；接口不可达时自动退回 TCP RTT 排序。
- 自定义列表写入 `/etc/xboxdownload/custom.<group>.txt`，重启后保留，IPK 将该目录列入 sysupgrade 配置备份。在线 IP 库缓存在 `/var/lib/xboxdownload/`，重启后可先用内置节点，启用定时同步后会自动刷新缓存。

## 在 NanoPi R5C 安装

以下以 OpenWrt **23.05.5**、目标 **rockchip/armv8** 为例。通过 `ubus call system board` 和 `opkg print-architecture` 核对固件目标与包架构；官方该目标使用 `aarch64_generic`。客户端需使用路由器 DNS，设备自带的 DoH 会绕过分流。

### 使用 IPK

1. 从仓库 Releases 页面选择同一版本的核心包 `xboxdownload_*_aarch64_generic.ipk`、界面包 `luci-app-xboxdownload_*_all.ipk`，以及可选中文包 `luci-i18n-xboxdownload-zh-cn_*_all.ipk`。也可使用下文的 SDK 自行构建。核对发布页 SHA256SUMS 中对应的校验值。
2. 将选中的包上传到路由器 /tmp，在路由器 SSH 终端执行下列命令。中文包按需安装。

```sh
opkg update
opkg install /tmp/xboxdownload_*_aarch64_generic.ipk /tmp/luci-app-xboxdownload_*_all.ipk
opkg install /tmp/luci-i18n-xboxdownload-zh-cn_*_all.ipk
/etc/init.d/rpcd reload
```

3. 重新登录 LuCI → **服务 → Xbox 下载 → 设置**。保留默认 dnsmasq 分流及 LAN 辅助 IP 模式，核对网桥名称，点击 **推荐 LAN 地址**。确认地址位于 DHCP 地址池之外，且未分配给休眠设备。勾选 **启用服务**，保存并应用。回退 DNS 应指向独立解析器，以免形成循环。
4. 在 **总览** 运行端到端自检。默认的 dnsmasq + 辅助 IP 模式共有七项检查。除三项本地 DNS 和两项本地 HTTP 检查外，还通过 dnsmasq 的 53 端口验证解析结果，并通过辅助 IP 的 80 端口验证 DNAT 后的 302。这两项分别随对应模式启用。随后在 Xbox 或 PC 发起下载，在命中日志中核对客户端 IP、Host、路径与目标，区分真实下载和路由器自检产生的记录。

### 手动部署二进制

下载对应版本的 `xboxdownloadd-aarch64-unknown-linux-musl`，将其放在同标签源码的仓库根目录。在电脑执行以下命令，并替换路由器地址。较新版 OpenSSH 的 scp 可通过 -O 与 Dropbear 配合。

```sh
router=root@192.168.1.1
ssh "$router" 'mkdir -p /tmp/xbd-install'
scp -O -r xboxdownloadd-aarch64-unknown-linux-musl data openwrt/xboxdownload/files "$router:/tmp/xbd-install/"
```

对照同标签的[核心包安装清单](openwrt/xboxdownload/Makefile)，将其中安装至 `/etc/hotplug.d/iface/95-xboxdownload` 的热插拔脚本上传为路由器的 `/tmp/xbd-install/95-xboxdownload`。然后在路由器上安装核心文件，保留已有配置，并将自定义 IP 目录加入升级备份。

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

在已有 LuCI 的路由器上，也可从电脑仓库根目录复制界面。手动复制安装英文界面；中文翻译包可由 SDK 构建。首次配置与自检步骤同上。

```sh
scp -O -r openwrt/luci-app-xboxdownload/htdocs/* "$router:/www/"
scp -O -r openwrt/luci-app-xboxdownload/root/usr/* "$router:/usr/"
ssh "$router" 'chmod 755 /usr/libexec/rpcd/luci.xboxdownload && rm -f /tmp/luci-indexcache.* && /etc/init.d/rpcd reload'
```

仅使用命令行时，在 `/etc/config/xboxdownload` 中填写实际的 `lan_device` 和已保留的 `alias_ip`，将 `main.enabled` 设为 1，再执行 `/etc/init.d/xboxdownload start`。

## 节点选择与日常检查

**节点测速**先测 TCP 延迟，再对入选节点发起 HTTP Range 下载。速度以 MB/s 显示，延迟以 ms 显示。可按运营商、地区或备注筛选，按速度或延迟排序，一键应用结果。`speed_mbps`、`last_speed_mbps` 和 `min_speed_mbps` 三个字段均以 MB/s（兆字节/秒）计。

**定时自动测速**默认启用，每 12 小时一次，首选本地时间凌晨 4 点。最近 15 分钟内有 302 命中时会延后测速。健康节点通常要求新节点快至少 20% 才切换，现有节点失效或速度低于 5 MB/s 时可提前替换。整组测试失败会保留已选 IP。应用节点时更新运行态、持久化 UCI 并刷新 dnsmasq 缓存。可调并发数、候选数、分片大小、超时和 IP 同步镜像（GitHub Raw、jsDelivr 或自定义地址）。

```sh
ubus call luci.xboxdownload status
ubus call luci.xboxdownload self_check
ubus call luci.xboxdownload logs '{"limit":10}'
logread -e xboxdownload
```

若自检通过但真实客户端未命中，从同一 LAN 客户端执行 `nslookup xvcf1.xboxlive.com <路由器地址>`，然后用 `curl -sD - -o /dev/null http://xvcf1.xboxlive.com/Z/routing/extraextralarge.txt` 核对 302 和 Location。自定义 HTTP 端口模式下，需自行确保目标 IPv4 的 TCP 80 流量能到达监听器。要停用，可在 LuCI 关闭总开关并应用，或执行 `/etc/init.d/xboxdownload stop` 临时停止。

## 开发与交叉编译

Rust 工作区位于 daemon/，核心包和 LuCI 包位于 openwrt/。在安装稳定版 Rust 的 Unix 开发机运行 `cargo test --workspace --locked`。精确域名、UCI 字段和 RPC 协议见[架构文档](docs/ARCHITECTURE.md)。

在 Linux x86_64 主机使用与目标固件一致的 [OpenWrt 23.05.5 rockchip/armv8 SDK](https://downloads.openwrt.org/releases/23.05.5/targets/rockchip/armv8/)。从仓库根目录开始，将 SDK 变量换成全新解压的 SDK 路径。

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

核心包调用 SDK 的目标编译器与链接器。已有静态目标二进制时，可向 make 传入 `XBOXDOWNLOAD_PREBUILT_BIN=/绝对路径/xboxdownloadd` 来复用。其他架构的目标三元组见[发布工作流](.github/workflows/release.yml)。本项目采用 [GNU GPL 第 3 版或更新版本](LICENSE)。
