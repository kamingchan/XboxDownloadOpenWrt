# XboxDownloadOpenWrt

**OpenWrt 路由器上的 Xbox / Microsoft Store 下载加速插件：在路由器上完成 CDN 测速、选点和 DNS 分流，全家主机和 PC 无需任何设置。**

[简体中文](README.md) · [English](README_EN.md) · [架构与接口契约](docs/ARCHITECTURE.md) · [部署案例](docs/CASES.md) · [开发指南](docs/DEVELOPMENT.md) · [贡献指南](CONTRIBUTING.md)

[![CI](https://github.com/kamingchan/XboxDownloadOpenWrt/actions/workflows/ci.yml/badge.svg)](https://github.com/kamingchan/XboxDownloadOpenWrt/actions/workflows/ci.yml)
[![GitHub Release](https://img.shields.io/github/v/release/kamingchan/XboxDownloadOpenWrt)](https://github.com/kamingchan/XboxDownloadOpenWrt/releases)
[![License: GPL-3.0-or-later](https://img.shields.io/badge/license-GPL--3.0--or--later-blue)](LICENSE)
![OpenWrt 23.05 | 24.10](https://img.shields.io/badge/OpenWrt-23.05%20%7C%2024.10-00B5E2?logo=openwrt&logoColor=white)
![Rust](https://img.shields.io/badge/daemon-Rust-orange?logo=rust)
![LuCI](https://img.shields.io/badge/UI-LuCI%20JS-2c3e50)
![Zero Telemetry](https://img.shields.io/badge/telemetry-zero-brightgreen)

XboxDownloadOpenWrt 把 [XboxDownload](https://github.com/skydevil88/XboxDownload) 的域名规则、测速思路和 IP 数据库搬到 OpenWrt 路由器上，以一个独立的 Rust 守护进程 + LuCI 插件运行。Xbox Series X|S、Xbox One、Windows PC 上的 Xbox App、Microsoft Store 和 PC Game Pass 下载，只要使用路由器 DNS，就会被引导到实测更快的国内 CDN 节点；路由器原有的 dnsmasq、SmartDNS、AdGuardHome、fw4 防火墙和 uhttpd 管理页面保持原样。

## 核心亮点

- **一台路由器服务全家**：Xbox 主机和 PC 不用改 DNS、不用常开电脑，接入 LAN 即生效。
- **轻量**：单个静态链接的 Rust 程序（aarch64 发布版约 2.2 MB），TLS 使用纯 Rust 的 `rustls`，不依赖 OpenSSL。
- **零侵入**：不占用 uhttpd 的 80 端口，不修改 fw4、SmartDNS、AdGuardHome 配置；只接管 Xbox 下载相关的精确域名。
- **自动选点**：两阶段测速（TCP 预筛 + HTTP Range 实测），空闲时定时复测，更快才切换，失败保留原节点。
- **可视化**：适配桌面和手机的 LuCI 面板，支持上游 DNS 自动检测、空闲 LAN 地址推荐、命中日志和一键 7 项端到端自检。
- **隐私**：零遥测，不上报任何测速结果、节点 IP 或访问日志。

## 目录

- [它是做什么的](#它是做什么的)
- [与原版方案对比](#与原版方案对比)
- [工作原理](#工作原理)
- [兼容性与架构](#兼容性与架构)
- [安装](#安装)
- [首次配置](#首次配置)
- [节点测速与日常使用](#节点测速与日常使用)
- [给 AI Agent 的快速部署指南](#给-ai-agent-的快速部署指南)
- [故障排查](#故障排查)
- [停用与卸载](#停用与卸载)
- [隐私声明](#隐私声明)
- [常见问题](#常见问题)
- [文档导航](#文档导航)
- [致谢与许可](#致谢与许可)

## 它是做什么的

Xbox 下载的默认 CDN 调度经常把国内用户分配到拥堵或境外节点。XboxDownloadOpenWrt 在路由器上做三件事：

1. **DNS 分流**：dnsmasq 只把 Xbox / Microsoft Store 下载相关的精确主机名转给本地守护进程，其他域名照旧走你现有的上游（SmartDNS、AdGuardHome、运营商 DNS 等）。
2. **HTTP 302 跳转**：把 `assets1.xboxlive.com` 等国际下载域名的 HTTP 请求原路径跳转到对应的 `.cn` 下载域名，路径和查询参数保持不变。
3. **CDN 选点**：对 CN1、CN2、Xbox App、Akamai 四组候选节点测速，把 `.cn` 下载域名解析到最快的节点。游戏文件由 CDN 直接传给主机或 PC，不经过路由器中转。

实际加速效果取决于运营商、地区线路和时段。

## 与原版方案对比

| 对比项 | 原版 XboxDownload（桌面端） | 手动在 OpenWrt 写 hosts / dnsmasq 规则 | **XboxDownloadOpenWrt** |
| :--- | :--- | :--- | :--- |
| 运行位置 | 一台常开的电脑，主机需把 DNS 指向这台电脑 | 路由器 | 路由器，所有 LAN 设备自动生效 |
| 主机 / PC 设置 | 每台设备改 DNS 或 hosts | 无 | 无，使用路由器 DNS 即可 |
| 节点选择 | 桌面端手动测速 | 手动查找 IP、手动更新 | 路由器后台自动测速，滞回防抖切换，失败保底 |
| 80 端口与 Web 管理 | 占用电脑的 80 端口 | 需要自行处理 HTTP 跳转 | 使用 LAN `/32` 辅助 IP + 独立 nftables DNAT，不占用 uhttpd 的 80 端口 |
| 对现有网络的影响 | 取决于 PC 的 DNS 服务 | 规则写宽了会误伤 Windows Update 等流量 | 只接管精确主机名；未选 IP 的分组不接管，Windows Update 继续走原解析链 |
| 与 SmartDNS / AdGuardHome / OpenClash | 不涉及 | 需要手工协调 | 不修改它们的配置；dnsmasq 使用本机 SmartDNS 时，插件默认跟随该上游 |
| IPv6 | 视设置而定 | 需手工屏蔽 AAAA | 按分组选择 IPv4 / IPv6 / 双栈，自动抑制可能绕过选点的记录 |
| 资源占用 | 桌面程序常驻运行 | 无额外进程 | 约 2.2 MB 静态 Rust 二进制，空闲时 CPU 占用接近零 |
| 验证手段 | 人工观察 | 人工 `nslookup` | 一键 7 项真实链路自检 + 命中日志 |

原版 XboxDownload 功能更全（例如覆盖更多平台、提供桌面端工具集），本项目专注于“路由器上的 Xbox / Microsoft Store 下载分流与选点”这一场景，并感谢原版提供的规则与数据。

### 详细优势

<details>
<summary><b>单一静态 Rust 守护进程</b></summary>

- 一个 `xboxdownloadd` 二进制同时提供 DNS、HTTP 302、测速、调度和本地控制接口，musl 静态链接，无运行时库依赖。
- HTTPS 使用 `rustls` + 内置 WebPKI 根证书，不依赖 OpenSSL；`rpcd` 直接调用 `xboxdownloadd ctl`，不需要 socat、python3 或 `nc -U`。
- 发布版使用 `opt-level = "z"`、LTO 和 strip，适合闪存较小的路由器。

</details>

<details>
<summary><b>零侵入路由器集成</b></summary>

- **不抢 80 端口**：默认在 LAN 接口上添加一个 `/32` 辅助 IP，只在独立的 `table inet xboxdownload` 中把发往该 IP 80 端口的流量 DNAT 到守护进程的 18080 端口。uhttpd、LuCI 登录页不受影响。
- **不动现有配置**：不改写 `/etc/config/uhttpd`、fw4 主表、SmartDNS 或 AdGuardHome 配置；dnsmasq 集成只是一个临时片段 `xboxdownload.conf`。
- **精确分流**：只匹配 12 个下载跳转域名、4 个 SRV 回退域名和已选节点分组的 `.cn` 下载域名，绝不使用 `xboxlive.com` 这类宽泛通配。
- **不提前劫持**：CDN 分组只有在选定了可用 IP（或启用了 Akamai 备用）后才接管；未测速的分组下，Windows Update 等请求仍走原有的 SmartDNS / OpenClash 解析链。
- **干净退出**：停止服务即删除辅助 IP、自有 nftables 表和 dnsmasq 片段；dnsmasq 只在片段增删改时重启一次，日常切换节点只发送 `SIGHUP` 清缓存。

</details>

<details>
<summary><b>自动化与交互体验</b></summary>

- **响应式 LuCI 面板**：总览、节点测速、设置三页，桌面和手机浏览器均可操作，提供简体中文翻译。
- **自动跟随上游 DNS**：`upstream_dns` 默认留空。服务启动时优先使用 dnsmasq 的全局 `server=`（如本机非 53 端口的 SmartDNS），其次使用 WAN 下发的 DNS，两者都没有可用地址时使用 `223.5.5.5:53`。LuCI 可显示检测结果，并支持手动覆盖。
- **空闲 LAN 地址推荐**：根据 LAN 网段、`ip neigh` 和 ping 探测推荐一个未被占用的高位地址作为辅助 IP。
- **两阶段测速**：先并发 TCP 延迟预筛，再对入选节点做 HTTP Range 实际下载测速；当前节点与新候选同批测量，避免不同时段结果不可比。支持 IPv4 / IPv6 / 双栈，可按运营商、地区、备注筛选。
- **智能定时切换**：默认每 12 小时测一次（优先凌晨 4 点）；最近 15 分钟有下载活动时自动延后；新节点至少快 20% 才切换；当前节点失效或低于 5 MB/s 时提前替换；整组测试失败时保留原节点。
- **7 项真实链路自检**：覆盖守护进程 DNS（A / AAAA / SRV）、HTTP 302（两类跳转）、经 dnsmasq 53 端口的实际解析，以及经辅助 IP 80 端口 DNAT 的实际跳转。

</details>

<details>
<summary><b>安全、隐私与兼容性</b></summary>

- **零遥测**：不收集、不上报任何数据，详见[隐私声明](#隐私声明)。
- **运行时路径硬编码**：控制套接字、数据目录、缓存目录和钩子脚本路径固定写在程序和 init 脚本中，不从 UCI 读取，LuCI 用户无法借配置改写路径提权。
- **全接口防 DNS 环路**：启动时比较上游地址与路由器所有接口地址（含 IPv4 映射的 IPv6 写法），拒绝会形成环路的配置。
- **BusyBox 兼容**：锁使用 `flock -n` 轮询实现，兼容 BusyBox 自带的 `flock`，不需要额外安装 util-linux `flock`，避免包冲突。
- **多版本固件**：自动发现 OpenWrt 23.05（`/tmp/dnsmasq.d`）与 24.10（`/tmp/dnsmasq.<cfg>.d`）的 dnsmasq `conf-dir`，支持多实例 dnsmasq；可用于官方 OpenWrt 及 FriendlyWrt、ImmortalWrt 等衍生固件。
- **配置不丢失**：自定义 IP 列表保存在 `/etc/xboxdownload/`，重启保留，IPK 已将该目录加入 sysupgrade 备份列表。

</details>

## 工作原理

```text
Xbox / PC ──DNS──► dnsmasq ──普通域名──► 原有 SmartDNS / AdGuardHome / 上游
                      └──下载及 SRV 域名──► xboxdownloadd :5353

Xbox / PC ──HTTP:80──► LAN 辅助 IP ──独立 DNAT──► xboxdownloadd HTTP :18080
          ◄──302 Location: http://assets1.xboxlive.cn/原路径──┘
          └──解析 .cn 下载域名──► 测速选定的 CDN 节点 ──► 直接下载
```

- 跳转域名的 A 记录返回辅助 IP，AAAA / HTTPS / SVCB 返回空的 NOERROR；指定的 SRV 回退域名返回空响应，防止下载中途回落到境外节点。
- 旧版 `dlassets` 请求跳转到 `dlassets.xboxlive.cn`；Xbox App 使用自己的下载域名，单独成组。
- 四个节点分组：CN1、CN2、Xbox App、Akamai（Akamai 默认关闭，可作为前三组的备用）。
- 可选“手动 DNS 模式”：不写 dnsmasq 片段，由 LuCI 生成 SmartDNS、AdGuardHome 或 dnsmasq 规则供复制。首次选定节点或更改参与分流的分组后需重新复制。
- 可选“按 MAC 屏蔽 IPv6”：仅在个别设备反复走缓存的 IPv6 路径时使用。

完整域名清单、UCI 字段、JSON 运行时配置和 ubus 接口见[架构与接口契约](docs/ARCHITECTURE.md)。

## 兼容性与架构

| 项目 | 要求 |
| :--- | :--- |
| 固件 | OpenWrt 23.05 / 24.10（使用 `opkg` 和 `.ipk`），以及基于它们的 FriendlyWrt、ImmortalWrt 等；需要 fw4（nftables）防火墙 |
| 依赖 | `nftables`（fw4 固件自带）、`kmod-nft-nat`、`ca-bundle`；LuCI 插件需要 `luci-base`（含 `ucode`） |
| 客户端 | 使用路由器下发的 DNS；设备或浏览器开启的 DoH / DoT 会绕过分流 |

发布页提供以下架构的核心包和静态二进制，LuCI 包与中文语言包为 `all` 架构，所有设备通用：

| `uname -m` | 常见设备 | 发布包架构 | 静态二进制 |
| :--- | :--- | :--- | :--- |
| `aarch64` | Rockchip、MediaTek Filogic、Amlogic 等 ARM64 软路由 / 路由器 | `aarch64_generic` | `xboxdownloadd-aarch64-unknown-linux-musl` |
| `x86_64` | x86 软路由、虚拟机 | `x86_64` | `xboxdownloadd-x86_64-unknown-linux-musl` |
| `armv7l` | IPQ40xx 等 ARMv7 硬浮点路由器 | `arm_cortex-a7_neon-vfpv4` | `xboxdownloadd-armv7-unknown-linux-musleabihf` |
| `mips` (小端 MT7621) | MT7621 系列路由器 | `mipsel_24kc` | `xboxdownloadd-mipsel-unknown-linux-musl` |

用 `opkg print-architecture` 查看固件接受的包架构。若列表中没有对应的发布包架构（例如 ARM64 设备显示 `aarch64_cortex-a53`），说明 CPU 指令集相同但包标签不同：可改用下文的[手动部署静态二进制](#方式二手动部署静态二进制)，或按[开发指南](docs/DEVELOPMENT.md)用对应目标的 SDK 构建。大端 MIPS（如 ath79）暂无发布包。

## 安装

### 方式一：安装 IPK（推荐）

1. 从 [Releases](https://github.com/kamingchan/XboxDownloadOpenWrt/releases) 页面下载同一版本的：
   - 核心包 `xboxdownload_<版本>_<包架构>.ipk`
   - 界面包 `luci-app-xboxdownload_<版本>_all.ipk`
   - 可选中文包 `luci-i18n-xboxdownload-zh-cn_<版本>_all.ipk`

   用发布页的 `SHA256SUMS` 核对文件。
2. 上传到路由器 `/tmp`，在路由器 SSH 终端执行（`<包架构>` 换成上表中的值）：

```sh
opkg update
opkg install /tmp/xboxdownload_*_<包架构>.ipk /tmp/luci-app-xboxdownload_*_all.ipk
opkg install /tmp/luci-i18n-xboxdownload-zh-cn_*_all.ipk   # 可选：中文界面
/etc/init.d/rpcd reload
```

### 方式二：手动部署静态二进制

适用于包架构标签不匹配或不想使用 opkg 的设备。在电脑上下载对应架构的 `xboxdownloadd-<目标三元组>`，并取得**同一标签**的源码（提供 `data/` 和 `openwrt/` 下的脚本），在仓库根目录执行：

```sh
router=root@192.168.1.1            # 换成你的路由器地址
bin=xboxdownloadd-aarch64-unknown-linux-musl   # 换成你的架构
ssh "$router" 'mkdir -p /tmp/xbd-install'
# 较新的 OpenSSH 需要 -O 以兼容 OpenWrt 的 Dropbear
scp -O -r "$bin" data openwrt/xboxdownload/files "$router:/tmp/xbd-install/"
```

然后在路由器上安装核心文件（保留已有配置，并把自定义 IP 目录加入升级备份）：

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

已有 LuCI 的路由器可从电脑仓库根目录复制界面文件（英文界面；中文语言包需通过 SDK 构建或安装发布页的 IPK）：

```sh
scp -O -r openwrt/luci-app-xboxdownload/htdocs/* "$router:/www/"
scp -O -r openwrt/luci-app-xboxdownload/root/usr/* "$router:/usr/"
ssh "$router" 'chmod 755 /usr/libexec/rpcd/luci.xboxdownload && rm -f /tmp/luci-indexcache.* && /etc/init.d/rpcd reload'
```

自行编译 IPK 或交叉编译二进制，请参阅[开发指南](docs/DEVELOPMENT.md)。

## 首次配置

1. 重新登录 LuCI，打开 **服务 → Xbox 下载 → 设置**。
2. 保留默认的 **由 dnsmasq 自动分流** 和 **LAN 辅助 IP** 模式，核对 LAN 接口名称（通常是 `br-lan`），点击 **推荐 LAN 地址**。确认该地址不在 DHCP 地址池内、也没有分配给休眠中的设备。
3. **回退 DNS 服务器** 默认留空，自动跟随 dnsmasq 上游。需要固定上游时，从列表选择或填写 `IPv4:端口` / `[IPv6]:端口`。清除已保存的地址并应用即可恢复自动模式。
4. 勾选 **启用服务**，保存并应用。
5. 打开 **节点测速**，对 CN1、CN2、Xbox App 分组分别测速并 **一键应用** 最快节点（或等待定时自动测速）。
6. 回到 **总览**，点击 **一键端到端自检**，默认模式下 7 项应全部通过。
7. 在 Xbox 或 PC 上开始下载，在总览的命中日志中核对客户端 IP、Host、路径与跳转目标，并区分路由器自检产生的记录（来源为路由器自身）。

典型硬件和复杂拓扑（单网口软路由、SmartDNS 共存、主路由 + 旁路由、多 VLAN / IPv6 双栈）的配置要点见[部署案例](docs/CASES.md)。

## 节点测速与日常使用

**节点测速**先测 TCP 延迟，再对入选节点发起 HTTP Range 下载。速度以 MB/s 显示，延迟以 ms 显示，可按运营商、地区或备注筛选，按速度或延迟排序，一键应用结果。`speed_mbps`、`last_speed_mbps` 和 `min_speed_mbps` 三个字段均以 **MB/s（兆字节/秒）** 计。

**定时自动测速**默认开启：每 12 小时一次，优先本地时间凌晨 4 点；最近 15 分钟内有下载跳转时延后；健康节点要求新节点至少快 20% 才切换；当前节点失效或低于 5 MB/s 时提前替换；整组失败保留已选 IP。应用节点会同时更新运行态、持久化 UCI 并刷新 dnsmasq 缓存。并发数、候选数、分片大小、超时和 IP 库同步镜像（GitHub Raw、jsDelivr 或自定义地址）均可在设置页调整。

常用命令：

```sh
ubus call luci.xboxdownload status
ubus call luci.xboxdownload self_check
ubus call luci.xboxdownload logs '{"limit":10}'
logread -e xboxdownload
```

## 给 AI Agent 的快速部署指南

> 本节面向代替用户操作路由器的 AI 编程助手（Claude Code、Codex、OpenClaw 等）。所有命令在**路由器 SSH 终端**中执行，除非特别注明“在电脑上”。每一步都有可检查的预期结果；结果不符时停止并向用户报告，不要猜测或强行继续。

### 0. 操作边界

- 只修改 `/etc/config/xboxdownload`，以及安装包本身带来的文件。**不要**修改 `/etc/config/uhttpd`、`/etc/config/firewall`、`/etc/config/dhcp`、SmartDNS 或 AdGuardHome 配置。
- 辅助 IP 必须是 LAN 网段内、DHCP 地址池外、当前无人使用的地址；不确定时询问用户。
- 上游 DNS 不能指向路由器自身的 53 端口或守护进程端口（默认 5353），init 脚本会拒绝启动。

### 1. 环境预检

```sh
cat /etc/openwrt_release                     # DISTRIB_RELEASE 应为 23.05.x 或 24.10.x
uname -m                                     # aarch64 / x86_64 / armv7l / mips
opkg print-architecture                      # 确认可接受的包架构
command -v nft fw4 ucode jsonfilter          # 四者都应存在（ucode 随 LuCI 提供）
ls /var/etc/dnsmasq.conf.*                   # 至少一个运行中的 dnsmasq 实例
ubus call network.interface.lan status | jsonfilter -e '@.l3_device' -e '@["ipv4-address"][0].address'
uci -q get dhcp.lan.start; uci -q get dhcp.lan.limit   # DHCP 地址池范围
df -h /overlay                               # 剩余空间应大于 4 MB
```

### 2. 架构映射

| `uname -m` | `opkg print-architecture` 含 | 选用 |
| :--- | :--- | :--- |
| `aarch64` | `aarch64_generic` | `xboxdownload_*_aarch64_generic.ipk` |
| `aarch64` | 其他 `aarch64_*` | 手动部署 `xboxdownloadd-aarch64-unknown-linux-musl` |
| `x86_64` | `x86_64` | `xboxdownload_*_x86_64.ipk` |
| `armv7l` | `arm_cortex-a7_neon-vfpv4` | `xboxdownload_*_arm_cortex-a7_neon-vfpv4.ipk` |
| `armv7l` | 其他 `arm_*` 硬浮点 | 手动部署 `xboxdownloadd-armv7-unknown-linux-musleabihf` |
| `mips` | `mipsel_24kc` | `xboxdownload_*_mipsel_24kc.ipk` |

LuCI 包和中文包始终使用 `_all.ipk`。安装步骤见[安装](#安装)。

### 3. 最小 UCI 配置

```sh
LAN_DEV=$(ubus call network.interface.lan status | jsonfilter -e '@.l3_device')
# 有 LuCI 插件时可让路由器推荐空闲地址；否则手动指定 DHCP 池外的空闲地址
ubus call luci.xboxdownload suggest_alias_ip "{\"lan_device\":\"$LAN_DEV\"}"

uci set xboxdownload.main.lan_device="$LAN_DEV"
uci set xboxdownload.main.alias_ip='192.168.1.253'      # 换成上一步确认的空闲地址
uci set xboxdownload.main.enabled='1'
uci commit xboxdownload
/etc/init.d/xboxdownload enable
/etc/init.d/xboxdownload restart
```

### 4. 选点

```sh
ubus call luci.xboxdownload speedtest_start '{"group":"xbox_cn1","auto_apply":true}'
ubus call luci.xboxdownload speedtest_status          # 重复执行直到 "running": false
# 依次对 xbox_cn2、xbox_app 执行；同一时间只能运行一个测速任务
ubus call luci.xboxdownload status                    # groups.*.selected_ipv4 应已填入
```

### 5. 验收

```sh
ubus call luci.xboxdownload self_check
```

预期 `"all_passed": true`，默认模式共 7 项：`dns_redirect_a`、`dns_redirect_aaaa`、`dns_srv_block`、`http_redirect`、`http_dlassets_redirect`、`dnsmasq_redirect_a`、`http_alias_redirect`。再执行：

```sh
nft list table inet xboxdownload                       # 存在 DNAT 规则
ip -4 addr show dev "$LAN_DEV" | grep "$(uci get xboxdownload.main.alias_ip)/32"
ls /tmp/dnsmasq*.d/xboxdownload.conf                   # 片段已写入
nslookup xvcf1.xboxlive.com 127.0.0.1                  # 返回辅助 IP
```

在 LAN 客户端上（可选，需用户配合）：`curl -sD - -o /dev/null http://xvcf1.xboxlive.com/Z/routing/extraextralarge.txt` 应返回 `302` 且 `Location` 指向 `http://assets1.xboxlive.cn/...`。

### 6. 回滚

```sh
/etc/init.d/xboxdownload stop          # 删除辅助 IP、nftables 表和 dnsmasq 片段
/etc/init.d/xboxdownload disable
uci set xboxdownload.main.enabled='0' && uci commit xboxdownload
# 完全卸载（可选）
opkg remove luci-i18n-xboxdownload-zh-cn luci-app-xboxdownload xboxdownload
# 验证回滚
nft list table inet xboxdownload 2>&1 | grep -q 'No such file' && echo 'nft clean'
ls /tmp/dnsmasq*.d/xboxdownload.conf 2>/dev/null || echo 'dnsmasq clean'
```

### 7. 排障速查

| 现象 | 检查 |
| :--- | :--- |
| 服务无法启动 | `logread -e xboxdownload`：常见原因是 `alias_ip` 为空或无效、`upstream_dns` 形成环路、端口冲突 |
| `dnsmasq_redirect_a` 失败 | dnsmasq `conf-dir` 未识别，或 DNS 重绑定保护拦截了私网应答；检查 `/var/etc/dnsmasq.conf.*` 中的 `conf-dir=` |
| `http_alias_redirect` 失败 | 辅助 IP 被网络重载移除，或 nftables 表缺失；执行 `/etc/init.d/xboxdownload restart` |
| 自检通过但主机未命中 | 客户端未使用路由器 DNS，或开启了 DoH；用 `logs` 确认是否有该客户端的记录 |

## 故障排查

若自检通过但真实客户端没有命中，在同一 LAN 的电脑上执行 `nslookup xvcf1.xboxlive.com <路由器地址>`，然后用 `curl -sD - -o /dev/null http://xvcf1.xboxlive.com/Z/routing/extraextralarge.txt` 核对 302 和 Location。常见原因：

- 主机或 PC 使用了自定义 DNS、DoH / DoT，未经过路由器 dnsmasq。
- 上游的透明代理或旁路由接管了 DNS，详见[部署案例](docs/CASES.md)。
- 使用“自定义 HTTP 端口”模式时，需要自行保证目标 IPv4 的 TCP 80 流量能到达监听端口。

## 停用与卸载

- 临时停用：在 LuCI 取消 **启用服务** 并应用，或执行 `/etc/init.d/xboxdownload stop`。
- 卸载：`opkg remove luci-i18n-xboxdownload-zh-cn luci-app-xboxdownload xboxdownload`。`/etc/xboxdownload/` 中的自定义 IP 列表保留，确认不再需要后可手动删除。

## 隐私声明

本项目**零遥测**：不收集、不上报测速结果、节点 IP、客户端信息或命中日志，所有日志只保存在路由器内存中。为实现功能，路由器会主动发起以下网络连接，全部可控：

| 连接 | 目的 | 如何关闭或替换 |
| :--- | :--- | :--- |
| GitHub Raw / jsDelivr 上的 `skydevil88/XboxDownload` IP 库 | 定时同步候选节点列表 | 设置页关闭 IP 库同步，或改用自定义镜像地址 |
| `xbox.skydevil.xyz` 包地址解析接口 | Xbox App 分组测速时获取真实下载包地址 | 为该分组填写自定义测速 URL；接口不可达时自动退回 TCP RTT 排序 |
| 候选 CDN 节点 | TCP 延迟和 HTTP Range 下载测速 | 关闭定时自动测速 |
| 回退上游 DNS（`upstream_dns`） | 解析受管域名中不需要改写的记录 | 默认跟随 dnsmasq 上游，其次是 WAN DNS，最后是 `223.5.5.5:53`。可填入信任的 DNS 固定上游 |

## 常见问题

<details>
<summary>会影响 Windows Update 或其他微软服务吗？</summary>

只接管精确的 Xbox / Microsoft Store 下载主机名。共享域名所在的 CDN 分组只有在你选定节点后才接管；未选节点时，这些请求照常走原有 DNS。

</details>

<details>
<summary>会占用路由器的 80 端口、影响 LuCI 登录吗？</summary>

不会。默认模式使用一个独立的 LAN `/32` 辅助 IP，只有发往该 IP 的 80 端口流量被 DNAT 到守护进程，路由器自身 LAN IP 的 80 端口仍由 uhttpd 提供。

</details>

<details>
<summary>已经在用 SmartDNS / AdGuardHome / OpenClash，还能用吗？</summary>

可以。插件不修改它们的配置。dnsmasq 仍是 LAN 客户端的 DNS 入口时，用默认的自动分流；如果客户端直接使用 AdGuardHome 等其他 DNS，可切换到手动模式并复制生成的规则。具体拓扑见[部署案例](docs/CASES.md)。

</details>

<details>
<summary>支持 OpenWrt 25.x（apk 包管理器）吗？</summary>

目前发布的是 `.ipk` 包，适用于使用 opkg 的 23.05 / 24.10。更新的固件可尝试手动部署静态二进制。

</details>

## 文档导航

| 文档 | 读者 | 内容 |
| :--- | :--- | :--- |
| [README.md](README.md) / [README_EN.md](README_EN.md) | 最终用户、用户的 AI Agent | 安装、配置、使用、排障 |
| [docs/CASES.md](docs/CASES.md) | 最终用户 | 脱敏的典型硬件与网络拓扑案例 |
| [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) | 开发者 | 域名规则、UCI / JSON / ubus 接口契约 |
| [docs/DEVELOPMENT.md](docs/DEVELOPMENT.md) | 开发者 | 构建、交叉编译、SDK 打包、测试与调试 |
| [CONTRIBUTING.md](CONTRIBUTING.md) | 贡献者 | Issue / PR 规范与检查清单 |
| [AGENTS.md](AGENTS.md) | 二次开发的 AI Agent 与开发者 | 仓库布局、架构不变量与验证命令 |

## 致谢与许可

- 域名规则、测速思路和 IP 数据库来自 [skydevil88/XboxDownload](https://github.com/skydevil88/XboxDownload)，感谢原作者与维护者。
- 本项目以 [GNU 通用公共许可证第 3 版或更新版本（GPL-3.0-or-later）](LICENSE) 发布。
- Xbox、Xbox Series X|S、Xbox One、Microsoft Store 和 PC Game Pass 是 Microsoft 的商标。本项目与 Microsoft 无关。
