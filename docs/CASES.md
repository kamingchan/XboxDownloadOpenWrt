# 部署案例 / Deployment Cases

[简体中文](#案例-1arm64-单网口软路由) · [English summary](#english-summary) · [返回 README](../README.md) · [Back to README_EN](../README_EN.md)

本文收录典型硬件与网络拓扑下的配置要点，帮助用户和 AI Agent 判断应该在哪台设备上安装、选择哪种模式。示例网络使用通用私网或文档保留地址（如 `192.168.1.1/24`、`10.0.0.1/24`、`127.0.0.1:6053`、`192.168.1.253`），不对应任何真实网络，请替换为你自己的值。

通用前提：Xbox / PC 使用的 DNS 必须最终经过安装了本插件的那台路由器上的 dnsmasq（自动模式），或你手动导入规则的 DNS 服务（手动模式）。

## 案例 1：ARM64 单网口软路由

**设备**：NanoPi R5C 等 Rockchip ARM64 设备，运行 OpenWrt 或 FriendlyWrt 23.05+。部分固件把 LAN 直接配置在物理网口 `eth1` 上，没有 `br-lan` 网桥。

**拓扑**：

```text
光猫 / 上级路由 ── eth0 (WAN) [软路由] eth1 (LAN, 192.168.1.1/24) ── 交换机 ── Xbox / PC
```

**要点**：

1. 架构：`uname -m` 为 `aarch64`。`opkg print-architecture` 包含 `aarch64_generic` 时直接安装发布页的 IPK；若只列出 `aarch64_cortex-a53` 等其他标签，改用 README 中的“手动部署静态二进制”。
2. LAN 接口名以实际三层设备为准，不要假设是 `br-lan`：

   ```sh
   ubus call network.interface.lan status | jsonfilter -e '@.l3_device'   # 例如输出 eth1
   ```

3. 在 LuCI 设置页把 **LAN 接口** 改为 `eth1`，再点击 **推荐 LAN 地址**；或用命令行：

   ```sh
   uci set xboxdownload.main.lan_device='eth1'
   uci set xboxdownload.main.alias_ip='192.168.1.253'   # DHCP 池外的空闲地址
   uci set xboxdownload.main.enabled='1'
   uci commit xboxdownload
   /etc/init.d/xboxdownload restart
   ubus call luci.xboxdownload self_check
   ```

4. 网口重新协商或执行 `ifup lan` 后，热插拔脚本会把 `/32` 辅助 IP 恢复到 `eth1`；之后改为网桥（如新增 Wi-Fi 后变成 `br-lan`）时，需要同步修改 `lan_device` 并重启服务。

## 案例 2：OpenWrt + SmartDNS 共存

**拓扑**：dnsmasq 仍监听 53 端口并为 LAN 提供 DNS，SmartDNS 监听本机非 53 端口（例如 `127.0.0.1:6053`），dnsmasq 把普通查询转发给 SmartDNS。

```text
Xbox / PC ──► dnsmasq :53 ──普通域名──► SmartDNS 127.0.0.1:6053 ──► 上游
                     └──Xbox 下载域名──► xboxdownloadd 127.0.0.1:5353
```

**要点**：

1. 保持默认的 **由 dnsmasq 自动分流** 模式，不需要修改 SmartDNS 配置。
2. **回退 DNS 服务器**（`upstream_dns`）默认留空。在此拓扑下，启动时会从 dnsmasq 的全局 `server=` 读到 `127.0.0.1:6053`，受管域名中需要转发的查询也走 SmartDNS。如果没有可用的全局上游，则使用 WAN DNS，最后回退到 `223.5.5.5:53`。仅在需要固定其他上游时，才在 LuCI 选择或填写地址。
3. 检测结果会自动排除路由器自身 53 端口和守护进程的 5353 端口，避免 DNS 环路。手动填写时同样不要使用 `127.0.0.1:53` 或 `192.168.1.1:53`。
4. 如果 SmartDNS 直接占用了 53 端口、dnsmasq 不再是 LAN 的 DNS 入口，自动模式不会生效。此时把 DNS 集成切换为 **手动配置分流**，复制 LuCI 生成的 SmartDNS 规则导入 SmartDNS；首次选定节点或更改参与分流的分组后需重新复制。AdGuardHome 作为 LAN DNS 入口时同理，使用生成的 AdGuardHome 规则。

## 案例 3：主路由 + 旁路由（透明代理 / Clash）

**拓扑**：主路由 `192.168.1.1` 负责拨号和 DHCP；旁路由 `192.168.1.2` 运行透明代理（如 OpenClash），部分设备把网关和 DNS 指向旁路由。

```text
Xbox ──网关/DNS──► 旁路由 192.168.1.2 (dnsmasq + 代理) ──► 主路由 192.168.1.1 ──► 互联网
```

**要点**：

1. **装在 Xbox 实际使用的 DNS 所在设备上**。Xbox 的 DNS 指向旁路由时装在旁路由，指向主路由时装在主路由。可在主机网络设置中查看 DNS，或在路由器上用 `ubus call luci.xboxdownload logs` 确认请求来源。
2. 辅助 IP 必须与 Xbox 处于同一网段（例如 `192.168.1.253`），并且在**两台**路由器的 DHCP 地址池之外。
3. 代理需要让 dnsmasq 保持为第一跳 DNS。如果代理通过防火墙劫持了 53 端口，或启用了 fake-ip 而没有为受管域名放行，dnsmasq 片段会被绕过。请在代理的 fake-ip 过滤列表中加入 `*.xboxlive.com`、`*.xboxlive.cn`、`*.delivery.mp.microsoft.com` 等受管域名，或改用不劫持 DNS 的模式。
4. 下载流量应直连。在代理规则中为 `xboxlive.cn`、`delivery.mp.microsoft.com` 和辅助 IP 设置 DIRECT，避免游戏文件经代理节点绕行。
5. 自检全部通过后，再从 Xbox 发起下载，确认命中日志中出现该主机的 IP。

## 案例 4：多 VLAN 或 IPv6 双栈环境

### 多 VLAN

**拓扑**：主 LAN `192.168.1.0/24`（`br-lan`），游戏 / IoT VLAN `10.0.0.0/24`（例如 `br-lan.20`），各自有 dnsmasq 实例或共享一个实例。

**要点**：

1. 插件会把 dnsmasq 片段写入所有运行中实例的 `conf-dir`，因此各 VLAN 的 DNS 分流都会生效。
2. 辅助 IP 模式的 DNAT 只匹配 `lan_device` 指定的入接口。最简单的做法是把 `lan_device` 设为 Xbox 所在的 VLAN 接口，例如 Xbox 在 `10.0.0.0/24` 时：

   ```sh
   uci set xboxdownload.main.lan_device='br-lan.20'
   uci set xboxdownload.main.alias_ip='10.0.0.253'
   uci commit xboxdownload && /etc/init.d/xboxdownload restart
   ```

3. 若多个 VLAN 都有需要加速的设备，可以：把它们放在同一 VLAN；或改用 **自定义 HTTP 端口** 模式，自行保证各 VLAN 访问目标 IPv4 的 TCP 80 能到达监听端口（包括该 VLAN 防火墙区域对路由器的入站放行）。
4. 访客区域默认拒绝访问路由器入站，DNAT 后的 18080 端口同样会被拒绝；不要把 Xbox 放在受限的访客区域。

### IPv6 双栈

**要点**：

1. LAN 保持 IPv6 即可。默认 IPv4 分组会为受管域名返回空的 AAAA / HTTPS / SVCB，防止客户端通过 IPv6 绕过选定节点。
2. 客户端必须使用路由器作为 IPv6 DNS。若路由器以中继模式下发运营商的 IPv6 DNS，客户端可能直接查询运营商 DNS 而绕过分流。在 **网络 → 接口 → LAN → DHCP 服务器 → IPv6 设置** 中确认路由器通告的是自身地址。
3. 运营商 IPv6 质量较好时，可在节点测速中把分组设为 `ipv6` 或 `dual` 并分别测速；仅 IPv4 的客户端请使用 `ipv4` 或 `dual`。
4. 个别设备反复使用缓存的 IPv6 路径时，再把它的 MAC 填入 **按 MAC 屏蔽 IPv6**（例如 `aa:bb:cc:dd:ee:ff`），该策略只作用于列出的设备。

---

## English summary

Example networks use generic private or documentation addresses. Substitute addresses from your own network. In every topology, the DNS server used by the console must be the router's dnsmasq running this plugin (automatic mode) or a DNS service into which you imported the generated rules (manual mode).

### Case 1: ARM64 single-LAN-port soft router

Devices such as the NanoPi R5C and other Rockchip ARM64 boards on OpenWrt or FriendlyWrt 23.05+. Some firmware puts LAN directly on `eth1` without a `br-lan` bridge. Use the `aarch64_generic` IPK if `opkg print-architecture` lists it, otherwise deploy the static binary. Read the real LAN device with `ubus call network.interface.lan status | jsonfilter -e '@.l3_device'`, set `lan_device` to `eth1`, pick an alias such as `192.168.1.253` outside the DHCP pool, enable, restart and run `self_check`. If LAN later becomes a bridge, update `lan_device` and restart.

### Case 2: OpenWrt with SmartDNS

When dnsmasq still listens on port 53 and forwards to SmartDNS on a local non-53 port (for example `127.0.0.1:6053`), keep automatic dnsmasq forwarding and leave **Fallback DNS server** (`upstream_dns`) empty. At startup the service uses the global dnsmasq `server=` entry for SmartDNS. If no global upstream is usable, it tries WAN DNS and falls back to `223.5.5.5:53`. Choose or enter an address in LuCI only to pin an override. Loopback/router addresses on port 53 or on the daemon port are filtered out to prevent loops. If SmartDNS or AdGuardHome itself serves port 53 to LAN clients, switch to manual forwarding and import the generated rules, re-copying them after groups first get nodes or the forwarded groups change.

### Case 3: Main router plus side router (transparent proxy / Clash)

Install the plugin on the device whose DNS the console actually uses. Choose an alias IP in the console's subnet that is outside the DHCP pools of both routers. Keep dnsmasq as the first DNS hop: avoid proxy modes that hijack port 53, or exclude the managed hosts (`*.xboxlive.com`, `*.xboxlive.cn`, `*.delivery.mp.microsoft.com`) from fake-ip. Route `xboxlive.cn`, `delivery.mp.microsoft.com` and the alias IP DIRECT so downloads do not go through proxy nodes.

### Case 4: Multiple VLANs or dual-stack IPv6

The dnsmasq fragment is installed into every running instance's `conf-dir`, but alias-mode DNAT matches only the `lan_device` ingress interface. Set `lan_device` and `alias_ip` to the console's VLAN (for example `br-lan.20` and `10.0.0.253`), keep consoles in one VLAN, or use custom HTTP port mode and make sure TCP 80 to the target reaches the listener from each zone. Restricted guest zones reject router input, including the DNATed port. For IPv6, keep LAN IPv6 enabled, make sure clients use the router as their IPv6 DNS rather than relayed ISP resolvers, test `ipv6` or `dual` groups if your IPv6 path is good, and use the per-MAC IPv6 block only for devices that keep using cached IPv6 routes.
