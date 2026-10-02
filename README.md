# 栖流 Qiliu · 轻量直播播放器

<p align="center">
  <img src="public/app-icon.svg" width="128" height="128" alt="栖流 Qiliu 直播播放器图标">
</p>

**在 macOS、Windows 和 Android 上，集中收藏并观看虎牙、哔哩哔哩（Bilibili / B站）、斗鱼、抖音直播和央视频道。**

栖流是一款开源直播播放器，适合长期观看固定直播间、希望界面简洁的用户。粘贴直播间链接即可播放，收藏常看的主播，下次打开继续播放上次的直播间。界面不包含弹幕、礼物和推荐信息流，同一时间播放一路直播。

**[下载最新发布版](https://github.com/Kihara-Ri/Qiliu/releases/latest)** · [全部版本与更新记录](https://github.com/Kihara-Ri/Qiliu/releases) · [反馈问题](https://github.com/Kihara-Ri/Qiliu/issues) · [English overview](#english-overview)

[快速上手](#快速上手) · [支持平台](#支持平台) · [常见问题](#常见问题) · [开发与构建](#开发与构建)

## 为什么选择栖流

- **专注看直播**：按需展开信息、设置、收藏与关于面板，让画面保持简洁。
- **集中管理直播间**：把不同平台的直播间加入本机收藏，查看主播名称、头像与直播状态。
- **打开即可继续**：记住上次播放的直播间和音量；播放与收藏是独立操作，临时换台不会自动加入收藏。
- **选择可用画质与线路**：以平台实际返回的选项为准；Bilibili 支持应用内扫码登录，使用账号可用的画质。
- **中断后尝试恢复**：区分线路续接、故障重连与换线，减少手动刷新。
- **保持最新**：改善体验的小补丁经签名校验后自动应用；应用新版本在「关于」中确认后一键更新，收藏与登录数据全程保留。
- **适配桌面与手机**：桌面支持全屏和音量控制；手机竖屏使用底部抽屉，横屏适配短视口。

## 下载与安装

前往 **[GitHub Releases](https://github.com/Kihara-Ri/Qiliu/releases/latest)**，展开页面中的 **Assets**，选择对应系统的安装包。具体版本、架构与可用文件以发布页为准。

| 系统 | 安装包名称格式 | 安装方式 |
| --- | --- | --- |
| macOS | `Qiliu-<版本号>-macOS-<架构>.dmg` | 打开 DMG，将“栖流”拖入 Applications |
| Windows x64 | `Qiliu-<版本号>-Windows-x64-setup.exe` | 下载后运行安装程序 |
| Android arm64 | `Qiliu-<版本号>-Android-arm64.apk` 或带 `-debug` 后缀的 APK | 下载后安装；首次侧载需允许当前来源安装应用 |

macOS 公开构建使用 ad-hoc 签名，尚未经过 Apple 公证；Windows 公开构建尚无代码签名证书，首次启动或安装时可能出现系统来源提醒。请确认安装包来自本仓库 Releases。

Android 文件名带 `-debug.apk` 的构建用于测试：同样是 release 编译，只是用调试密钥签名。正式签名与调试签名的安装包可能无法互相覆盖安装，更新前请查看对应 Release 说明。

## 快速上手

1. 安装并打开栖流，粘贴受支持的直播间链接。
2. 点击“立即播放”开始观看；想保留这个直播间时，再点击“收藏”。
3. 在“设置”中选择当前可用的画质与线路；在“信息”中查看播放状态。
4. 下次启动会尝试播放上次的直播间，也可以从“收藏”切换。

桌面端支持 `⌘V` / `Ctrl+V` 粘贴链接，双击画面或按 `F` 切换全屏。音量按钮可调节音量与静音。

## 支持平台

| 直播平台 | 支持的输入 | 说明 |
| --- | --- | --- |
| 虎牙 | `huya.com` 直播间链接 | 支持直播及平台提供的回放，区分直播、回放与离线状态 |
| 哔哩哔哩 / Bilibili / B站 | `live.bilibili.com` 直播间链接 | 支持扫码登录；画质取决于账号权限与直播间实际提供的档位 |
| 斗鱼 | 数字房间、房间别名、带 `rid` 的活动链接 | 画质与 CDN 线路以平台返回值为准 |
| 抖音 | `live.douyin.com` 直播间链接、`v.douyin.com` 分享短链接 | 若短链接未指向可解析的直播间，请使用直播间直链 |
| 央视频道 | `tv.cctv.com/live/...` 央视官网直播链接 | 目前支持 CCTV-1 综合与 CCTV-13 新闻；其余频道需平台授权，暂不在公开线路提供 |

应用可从分享文案中提取受支持的链接。平台验证、地区限制、付费权限或接口变更可能导致部分直播间无法解析。

## 常见问题

### Mac 上可以用栖流看 B站、虎牙或斗鱼直播吗？

可以。栖流面向 macOS、Windows 与 Android，支持虎牙、Bilibili、斗鱼、抖音直播和央视频道（CCTV-1、CCTV-13）。请从发布页选择适合设备架构的安装包。

### 支持弹幕、多开或搜索主播吗？

目前不支持弹幕、礼物、主播搜索、平台推荐或多路同时播放。栖流适合通过链接打开直播间，以及在已收藏的直播间之间切换。

### 必须登录吗？登录后一定有原画吗？

Bilibili 支持扫码登录，以获取账号实际可用的画质。最高档位还取决于主播推流和平台策略，登录不保证每个直播间都有相同画质。斗鱼与抖音的解析无需用户在应用中配置账号 Cookie。

### 收藏和登录信息保存在哪里？

收藏、上次播放项和音量保存在本机。Bilibili 登录凭据由 Rust 端处理，保存于当前设备的系统安全存储；不进入页面 DOM、收藏数据或应用日志。退出登录会删除当前设备保存的凭据。

### 应用会自动更新吗？

会分两种情况：改善体验的小改动以「体验补丁」形式自动应用（可在「设置 → 更新」关闭）；完整的新版本会在「关于」面板提示，由你确认后再下载安装（Android 会引导到发布页）。更新只替换应用本体，收藏、上次播放、音量与登录信息都保留在原来的位置，不会丢失。

### 卡顿或打不开直播间怎么办？

先确认直播间正在开播，并检查网络；再尝试切换可用线路或降低画质。自动恢复无法消除主播推流故障、平台异常或本地网络中断。

如果问题持续，请通过 [Issues](https://github.com/Kihara-Ri/Qiliu/issues) 提供应用版本、操作系统、直播间链接、复现步骤和错误提示。请勿附上 Cookie 或其他登录凭据。

### 栖流是开源软件吗？

是。项目使用 [AGPL-3.0-only](LICENSE) 许可证，源码和构建说明公开，第三方依赖许可见 [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)。

## English overview

Qiliu (栖流) is an open-source, lightweight live stream player for macOS, Windows, and Android. It supports Huya, Bilibili, Douyu, and Douyin live rooms, plus CCTV channels (CCTV-1 and CCTV-13).

Paste a live room URL to watch, save favorite rooms across platforms, and resume the last room when reopening the app. Qiliu focuses on one stream at a time, with selectable quality and playback lines where available. It does not include bullet chat, gifts, streamer search, or a recommendation feed.

[Download releases](https://github.com/Kihara-Ri/Qiliu/releases/latest) · [Report an issue](https://github.com/Kihara-Ri/Qiliu/issues) · [License: AGPL-3.0-only](LICENSE)

## 开发与构建

技术栈：**Tauri 2、Rust、TypeScript、mpegts.js、HLS.js**。

### 从源码运行


桌面端准备环境：

- Node.js 20 LTS 或更高版本；
- Rust stable；
- macOS 需要 Xcode Command Line Tools；Windows 需要 Microsoft C++ Build Tools 与 WebView2。

```bash
git clone https://github.com/Kihara-Ri/Qiliu.git
cd Qiliu
npm ci
npm run tauri dev
```

首次运行会自动展开设置面板。可使用以下链接测试：

```text
https://www.huya.com/196645
https://live.bilibili.com/22907643
https://tv.cctv.com/live/cctv1/
```

## 编译与验证

完整自动化检查：

```bash
npm run check
```

该命令依次运行：

- TypeScript 收藏与恢复策略测试；
- TypeScript 类型检查和 Vite 生产构建；
- Rust 房间解析、链接校验、画质选择、签名与登录数据处理测试。

当前桌面系统一键检查并打包：

```bash
npm run package:current
```

构建 macOS 应用：

```bash
npm run tauri build -- --bundles app
```

输出位置：

```text
src-tauri/target/release/bundle/macos/栖流.app
```

生成带 Applications 快捷方式的 DMG：

```bash
npm run package:macos
```

脚本会执行 ad-hoc 签名验证、创建压缩 DMG 并输出 SHA-256。安装包位于：

```text
release/栖流 <版本号>.dmg
```

Windows x64 NSIS 安装程序（需在 Windows 运行）：

```bash
npm run package:windows
```

Android arm64 调试 APK（需 Android SDK、NDK 和 JDK 17）：

```bash
npm run package:android
```

准备新版本并同步所有版本号（以下以 `1.3.2` 为示例，请替换为待发布版本）：

```bash
npm run release:prepare -- 1.4.1
```

推送对应的 `v1.4.1` 标签后，GitHub Actions 会在原生 macOS、Windows 与 Android 环境构建并把安装包集中发布到同一个 Release。提交、构建、上传与下载核验见 [完整发布流程](docs/release-process.md)。完整环境、签名和故障排查见 [跨平台界面与构建](docs/cross-platform-builds.md)。

真实直播长时间播放测试依赖当前房间状态、平台接口和网络环境，不包含在默认自动化测试中。发布新版本前应分别用一个正在直播的虎牙和 Bilibili 房间进行持续播放验证。

## 项目结构

```text
src/
  main.ts                    面板、收藏、账号与音量交互
  playback-supervisor.ts     播放、欠缓冲检测、重连与换线
  playback-recovery-policy.ts 恢复策略状态机
  source-library.ts          链接清洗、收藏持久化与旧数据迁移
src-tauri/src/
  huya.rs                    虎牙房间、直播线路和动态签名
  huya_wup.rs                虎牙回放 WUP、画质选择与时间同步
  bilibili.rs                Bilibili 房间、真实 QN 与线路解析
  bilibili_auth.rs           扫码登录、Cookie 白名单与系统钥匙串
  douyu.rs                   斗鱼房间、签名与播放线路
  douyin.rs                  抖音房间与播放线路
  douyin_sign.rs             抖音请求签名
  cctv.rs                    央视频道表、CDN HLS 画质校验
  live_source.rs             直播来源与链接处理
  stream.rs                  跨平台统一播放信号
  lib.rs                     Tauri 命令入口与平台路由
docs/
  huya-stream-source-system.md 虎牙完整技术与故障记录
  bilibili-and-favorites.md    Bilibili 画质、登录与收藏说明
  settings-panel-layout.md     面板布局和合成约束
  cross-platform-builds.md     移动端布局、三平台构建与自动发布
```

## 技术原理


### 1. 信号解析在 Rust 端完成

前端只接收经过校验的统一播放信息，不直接持有登录 Cookie，也不自行拼接平台签名。

```text
直播间链接
    ↓
Rust 平台解析器
    ├─ 虎牙：房间信息、动态签名、CDN 列表、WUP 回放
    ├─ Bilibili：真实房间号、账号档位、H.264 FLV / HLS 线路
    ├─ 斗鱼：房间信息、动态签名、画质与 CDN
    ├─ 抖音：分享链接、房间信息、H.264 FLV / HLS 线路
    └─ 央视频道：CDN HLS 画质表与线路校验
    ↓
统一 StreamSignal
    ↓
前端播放与恢复监督器
```

虎牙直播优先选择原画 H.264 FLV；回放通过 WUP 获取网页同源录像，并根据 `videoSyncTime` 对齐当前回放位置。Bilibili 会先读取 `accept_qn`，请求其中最高的 H.264 FLV，再用服务器返回的 `current_qn` 标记实际画质。

### 2. FLV 只做解复用，视频交给系统解码

FLV 使用 `mpegts.js` 解复用到 Media Source Extensions；HLS 使用 `HLS.js` 播放。应用不打包 FFmpeg、VLC 或 libmpv，H.264 最终由各系统 WebView 的媒体链路解码。

这种方案安装体积小、前端可控，也便于观察缓冲、帧率和掉帧；代价是必须认真处理 WebKit MSE 的缓冲边界和原生视频层合成行为。

### 3. 恢复策略区分“续接、重连、换线”

`src/playback-supervisor.ts` 不会在一次轻微波动后立刻更换 CDN：

1. 媒体时间与实际解码帧同时停止 12 秒后，才尝试软追帧；最长 30 秒才认定硬卡顿。
2. 正常 EOF 或首次故障先刷新当前线路的播放地址和签名。
3. 同一线路在 30 秒内连续失败两次，才轮换 CDN。
4. 重试采用 1、2、4、8、15 秒的有界退避，之后保持 15 秒重试。
5. Bilibili 只有在 90 秒内出现 3 次真实欠缓冲，或欠缓冲同时伴随明显掉帧时，才在当前会话降档。

监控面板会分别显示同线路续接、故障重连与真正换线次数，避免把平台会话自然结束误判成网络故障。

### 4. 账号和本机数据分层保存

- 收藏、上次播放项和音量保存在本机 `localStorage`。
- Bilibili Cookie 只由 Rust 处理，并保存在 Apple 钥匙串、Windows 凭据管理器或 Android Keystore 加密的安全存储中。
- Cookie 不进入 DOM、收藏数据或应用日志。
- 退出登录只删除当前设备的安全凭据，不影响网页端或其他设备。

为保证从旧版 Simple Live 覆盖升级后数据连续，Bundle ID、旧 `localStorage` key 和钥匙串 service 暂时保留为兼容层；对外产品名称已经统一为“栖流 / Qiliu”。

### 斗鱼与抖音直播源

支持斗鱼数字房间、房间别名及带 `rid` 的活动链接；抖音支持 `live.douyin.com` 房间链接及 `v.douyin.com` 分享短链接。链接可直接播放或加入本机收藏，收藏列表复用直播状态查询。短链接若跳转到不提供直播房间资料的页面，会提示使用直播间直链。

斗鱼从房间接口读取开播状态，在应用内执行官方动态签名脚本，再请求 H5 播放地址；画质和 CDN 线路以平台返回值为准。抖音使用匿名访客 Cookie、a_bogus 签名接口及页面 roomStore 备用解析，提取 H.264 FLV/HLS 画质。无需安装 Python 或 Node.js，也不内置参考项目中的账号 Cookie。平台验证、地区限制、付费权限或接口变更仍可能导致解析失败；此时不会显示为未开播。

## 央视频道直播源

支持粘贴央视官网直播链接（如 `https://tv.cctv.com/live/cctv1/`）。解析器读取央视网网页播放器使用的公开 CDN HLS：无需登录、Cookie 或签名，画质表从 CDN master 播放列表动态解析，并逐档校验真实可用后才显示；当前提供 1080P 至 480P 多档 H.264 画质。

需要说明范围：该 CDN 的频道模板覆盖大部分央视频道，但实际携带直播流的目前只有 **CCTV-1 综合**与 **CCTV-13 新闻**——这两个频道是网页播放器的全球保底线路。其余频道（体育、电影等）的正式播放地址由平台接口按地区与版权授权下发，非授权客户端只会得到混淆数据，因此暂不支持；粘贴这类频道链接会提示「该央视频道暂不受支持」，不会误报为未开播。

## 验证范围

自动化构建通过不等于真实设备长时间播放通过。Windows 和 Android 仍需分别完成直播、回放、扫码登录、休眠恢复和弱网验收。

手动网络检查（默认测试不会访问真实直播平台）：

```bash
QILIU_LIVE_PROBE=https://www.douyu.com/9999 cargo test --manifest-path src-tauri/Cargo.toml probe_live_source -- --ignored --nocapture
QILIU_LIVE_PROBE=https://live.douyin.com/房间号 cargo test --manifest-path src-tauri/Cargo.toml probe_live_source -- --ignored --nocapture
QILIU_LIVE_PROBE=https://tv.cctv.com/live/cctv1/ cargo test --manifest-path src-tauri/Cargo.toml probe_live_source -- --ignored --nocapture
```

该检查验证解析与媒体首段数据；客户端画面、声音、画质切换和长时间播放仍需在对应系统实际验收。

## 进一步阅读

- [虎牙直播源系统技术文档](docs/huya-stream-source-system.md)
- [Bilibili 与直播收藏技术说明](docs/bilibili-and-favorites.md)
- [右侧面板布局说明](docs/settings-panel-layout.md)
- [跨平台界面与构建](docs/cross-platform-builds.md)

## 参考与许可

虎牙、Bilibili 解析和长时间播放策略参考了 [liuchuancong/pure_live](https://github.com/liuchuancong/pure_live) 的公开实现，并重新收敛为单窗口、单路播放、无弹幕的 Tauri 架构。FLV 解复用使用 Apache-2.0 的 [mpegts.js](https://github.com/xqq/mpegts.js)。

本项目以 [AGPL-3.0-only](LICENSE) 发布。第三方依赖许可见 [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)。

斗鱼与抖音解析实现参考 [DouyinLiveRecorder](https://github.com/ihmily/DouyinLiveRecorder) 的公开实现，相关许可证见 [第三方声明](THIRD_PARTY_NOTICES.md)。
