# VpnBrowser

一个内存占用极低的 Windows 浏览器，**内置 mihomo (Clash.Meta) 代理核心**，支持导入 Clash Verge Rev 的配置与节点，代理只作用于浏览器自身流量，不影响系统其它程序。

## 特性

- 基于 Tauri v2 + 系统 WebView2，内存占用远低于 Chromium/Electron 系浏览器
- **多标签页**：每个标签一个独立 WebView，可任意新建 / 切换 / 关闭
- **无痕标签**：基于 WebView2 InPrivate，不落盘缓存与 Cookie，关闭即清除
- **WebRTC 防泄漏**：强制 `disable_non_proxied_udp`，WebRTC 只走代理出口，不暴露真实 IP
- 内置 mihomo 核心（sidecar 打包），支持 ss / vmess / vless / trojan / hysteria / hysteria2 / tuic 等全部 Clash 节点协议
- 导入方式：订阅链接 / 直接粘贴 Clash YAML
- 代理组（Selector / URLTest / Fallback）节点切换
- 直连 / 规则 / 全局 三种模式一键切换
- 仅监听 `127.0.0.1`，不设置系统代理、不启用 TUN，**只影响浏览器**

## 架构

```
ui/            前端界面（地址栏、设置面板、节点选择）
src-tauri/     Rust 后端
  src/lib.rs   窗口 + 多 WebView 布局 + mihomo sidecar 管理 + 代理 API
  binaries/    mihomo-<target-triple>.exe（CI 自动下载，不入库）
.github/       云端构建 Windows 安装包
```

每个标签页对应一个 `content-<id>` 子 WebView，活动标签显示在工具栏下方，
其余标签移出可视区域；无痕标签在创建时开启 `incognito`。

浏览器通过 WebView2 环境变量
`--proxy-server=socks5://127.0.0.1:<port> --force-webrtc-ip-handling-policy=disable_non_proxied_udp`
把流量指向内置 mihomo 的混合端口，从而做到「只代理浏览器」且防 WebRTC 泄漏。

## 本地构建

```powershell
npm install
# 手动放置 mihomo 二进制
#   src-tauri/binaries/mihomo-x86_64-pc-windows-msvc.exe
npm run tauri build
```

## 云端构建

推送到 `main` 或手动触发 `build` workflow，产物在 Artifacts 里下载。
