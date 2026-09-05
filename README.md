# 栖流 Qiliu

<p align="center">
  <img src="public/app-icon.svg" width="128" height="128" alt="栖流应用图标">
</p>

栖流是一款面向 macOS、Windows 与 Android 的轻量直播播放器。它只保留观看所必需的内容：打开应用自动播放、一个按需出现的设置层、直播收藏和音量控制。

当前版本：**1.2.0**
支持来源：**虎牙、Bilibili**
技术栈：**Tauri 2、Rust、TypeScript、mpegts.js、HLS.js**

## 它解决什么问题

平台网页播放器适合浏览内容，但长期观看固定直播间时，菜单、推荐、弹幕和复杂状态会占用大量空间。栖流把重点放在另一组问题上：尽快出画、使用真实可用的最高画质、减少周期性中断，并在外部直播线路发生变化时恢复播放。

| 问题 | 原因 | 栖流的处理方式 |
| --- | --- | --- |
| 虎牙每隔一段时间短暂停顿或换线 | 部分 FLV 长连接会自然结束；旧签名也会过期 | 提前获取当前线路的新签名；正常 EOF 在同一 CDN 续接，只有连续故障才换线 |
| 播放器请求了“原画”，实际却不是最高画质 | 平台可能下调未登录请求；请求档位不等于实际档位 | 先读取可用档位，再请求最高 H.264 FLV，并以响应的 `current_qn` 为最终依据 |
| Bilibili 原画频繁欠缓冲 | 高码率 FLV、WebKit MSE 与网络抖动共同影响 | Worker 解复用、启动缓冲和真实欠缓冲检测；只有持续不稳定时才在本次会话降到 QN 400 |
| 未直播时播放旧的低清片段 | 直播、回放和离线状态混淆，或旧录像被循环使用 | 区分 `LIVE`、`REPLAY`、`OFFLINE`；回放重新解析当前录像、画质和同步位置 |
| 打开设置面板出现黑块并挤压视频 | macOS WebKit 原生视频层在面板卸载、重新布局时发生合成抖动 | 面板永久挂载，只通过 `transform` 移出视口，不改变视频层尺寸 |
| 分享文案无法直接粘贴 | 链接夹带查询参数、零宽字符和两侧标点 | 支持 `⌘V` / `Ctrl+V`，自动提取并规范化第一个受支持的直播间链接 |

## 使用体验

- 记住最后播放的直播间，启动后直接播放。
- 收藏多个虎牙或 Bilibili 直播间，但同一时间只播放一路。
- 获取主播名称、头像和直播状态。
- “立即播放”和“收藏”是两个独立动作；换源不会自动扩充收藏列表。
- 在设置页查看当前直播间，并手动选择平台实际返回的画质与线路。
- 双击画面或按 `F` 进入、退出全屏。
- 左下角音量按钮支持静音，并按低、中、高三级显示声波。
- 设置、收藏、监控和关于彼此分离；调试信息不会占据日常界面。
- Bilibili 支持应用内扫码登录，以获取账号实际可用的更高画质。
- 手机竖屏使用底部抽屉和底部功能标签；横屏短视口自动切换为全屏设置层。
- 所有移动端主要控件至少 44px，触屏打开面板时不会自动唤起软键盘。
- Windows 使用无边框窗口，最小化、最大化/还原和关闭操作位于应用内部。

## 技术原理

### 1. 信号解析在 Rust 端完成

前端只接收经过校验的统一播放信息，不直接持有登录 Cookie，也不自行拼接平台签名。

```text
直播间链接
    ↓
Rust 平台解析器
    ├─ 虎牙：房间信息、动态签名、CDN 列表、WUP 回放
    └─ Bilibili：真实房间号、账号档位、H.264 FLV 线路
    ↓
统一 StreamSignal
    ↓
前端播放与恢复监督器
```

虎牙直播优先选择原画 H.264 FLV；回放通过 WUP 获取网页同源录像，并根据 `videoSyncTime` 对齐当前回放位置。Bilibili 会先读取 `accept_qn`，请求其中最高的 H.264 FLV，再用服务器返回的 `current_qn` 标记实际画质。

### 2. FLV 只做解复用，视频交给系统解码

直播使用 `mpegts.js` 将 FLV 解复用到 Media Source Extensions；回放使用 `HLS.js`。应用不打包 FFmpeg、VLC 或 libmpv，H.264 最终由各系统 WebView 的媒体链路解码。

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

## 安装

### macOS

1. 从本仓库的 **Releases** 页面下载最新的 `Qiliu-x.y.z-macOS.dmg`。
2. 打开 DMG，将“栖流”拖入 `Applications`。
3. 首次启动时粘贴虎牙或 Bilibili 直播间链接并点击“立即播放”；需要保留时再单独收藏。

当前公开构建使用本地 ad-hoc 签名，尚未经过 Apple 公证。如果 macOS 阻止首次启动，请在 Finder 中右键应用并选择“打开”，确认应用来源后再启动。

### Windows

从 **Releases** 下载 `Qiliu-x.y.z-Windows-x64-setup.exe` 并运行。当前 NSIS 安装程序按当前用户安装，不要求系统级目录写入权限；公开构建尚未购买 Windows 代码签名证书，SmartScreen 可能显示来源提醒。

### Android

从 **Releases** 下载 `Qiliu-x.y.z-Android-arm64.apk`。系统首次侧载时会要求允许当前文件管理器或浏览器安装未知应用。

文件名带 `-debug.apk` 的构建使用调试签名，只用于测试；固定 release keystore 签名的 APK 才支持后续版本原地覆盖升级。

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
release/栖流 1.2.0.dmg
```

Windows x64 NSIS 安装程序（需在 Windows 运行）：

```bash
npm run package:windows
```

Android arm64 调试 APK（需 Android SDK、NDK 和 JDK 17）：

```bash
npm run package:android
```

准备新版本并同步所有版本号：

```bash
npm run release:prepare -- 1.2.0
```

推送对应的 `v1.2.0` 标签后，GitHub Actions 会在原生 macOS、Windows 与 Android 环境构建并把安装包集中发布到同一个 Release。完整环境、签名和故障排查见 [跨平台界面与构建](docs/cross-platform-builds.md)。

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
  stream.rs                  跨平台统一播放信号
  lib.rs                     Tauri 命令入口与平台路由
docs/
  huya-stream-source-system.md 虎牙完整技术与故障记录
  bilibili-and-favorites.md    Bilibili 画质、登录与收藏说明
  settings-panel-layout.md     面板布局和合成约束
  cross-platform-builds.md     移动端布局、三平台构建与自动发布
```

## 已知边界

- 当前只解析虎牙和 Bilibili，不支持弹幕、礼物、搜索或平台推荐。
- Bilibili 的最高可用档位由账号权限、主播推流和平台策略共同决定；登录并不保证所有房间都提供相同画质。
- 自动恢复可以减少短暂中断，但无法消除主播推流故障、平台服务异常或本地网络中断。
- 自动化构建通过不等于真实设备长时间播放通过；Windows 和 Android 仍需分别完成直播、回放、扫码登录、休眠恢复和弱网验收。

## 进一步阅读

- [虎牙直播源系统技术文档](docs/huya-stream-source-system.md)
- [Bilibili 与直播收藏技术说明](docs/bilibili-and-favorites.md)
- [右侧面板布局说明](docs/settings-panel-layout.md)
- [跨平台界面与构建](docs/cross-platform-builds.md)

## 参考与许可

虎牙、Bilibili 解析和长时间播放策略参考了 [liuchuancong/pure_live](https://github.com/liuchuancong/pure_live) 的公开实现，并重新收敛为单窗口、单路播放、无弹幕的 Tauri 架构。FLV 解复用使用 Apache-2.0 的 [mpegts.js](https://github.com/xqq/mpegts.js)。

本项目以 [AGPL-3.0-only](LICENSE) 发布。第三方依赖许可见 [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)。
