# Twitch 原生 Rust 解析器可行性与实施设计

- 日期：2026-10-15（日本时间）
- 目标项目：栖流 Qiliu v1.5.0（Tauri 2 / Rust / TypeScript / HLS.js）
- 状态：**已实现（`src-tauri/src/twitch.rs`）：macOS 以 jankos 实测播放正常；Windows / Android 的 playlist 经 Rust IPC 取回，待真机验收**
- 研究范围：Twitch 公开频道直播；VOD、Clip、订阅用户观看权限不在首版范围
- 测试环境：Mac（Gigachad.local），目标 `https://www.twitch.tv/solarbacca`
- 安全：本文不保存任何真实 Twitch 播放令牌、签名、频道 Cookie 或具体的带签名 CDN 地址。

## 一、结论

可以在 `src-tauri/src/twitch.rs` 中复用项目已有的 `reqwest`、`serde_json`、`url` 和 `LiveStream` 实现 Twitch 直播源解析，不依赖 Streamlink、Python 或 FFmpeg。已实测以下链路成功：

1. Twitch GraphQL `ChannelShell`：HTTP 200，频道存在。
2. Twitch GraphQL `StreamMetadata`：HTTP 200，测试时 `stream` 非空。
3. Twitch GraphQL `PlaybackAccessToken`：HTTP 200，返回 `signature` / `value`。
4. `usher.ttvnw.net` 的 `/api/v2/channel/hls/{channel}.m3u8`：HTTP 200、`#EXTM3U`，共 6 个 HLS variant。
5. 1080p60 variant 的媒体播放列表：在 `Origin: tauri://localhost` 下 HTTP 200；在 `Origin: http://tauri.localhost` 下反复 HTTP 403。
6. 实际 TS 分片的区间请求：在上述两种 Origin 下均得到 HTTP 206，并带 `Access-Control-Allow-Origin: *`。

**未验证**：完整播放器渲染、视频画面和声音、长期稳定性、Windows/Android 真机与广告插入时的表现。Twitch API 属于网页内部接口，不能当作官方稳定的第三方取流 API。

## 二、端到端路径

```text
Twitch 频道 URL
  -> 仅接受 twitch.tv 指定域名和合法 login，规范化频道 slug
  -> TwitchClient::metadata(channel) / is_live(channel)
  -> POST https://gql.twitch.tv/gql (PlaybackAccessToken)
     -> 得到 signature/value（只保存在内存，禁止日志）
  -> GET https://usher.ttvnw.net/api/v2/channel/hls/{channel}.m3u8
     -> Rust 解析 variant 的 codecs / resolution / frame rate / bandwidth
  -> 选择 H.264 variant 或保留 master 以支持 ABR
  -> LiveStream { platform: "twitch", format: "hls", ... }
  -> 现有 playback-supervisor.ts + HLS.js
  -> 对 WebView 的 playlist 403：考虑仅用受限 Rust IPC 代理 HLS 文本清单
  -> 媒体 TS 分片尽量由 HLS.js 直接从 Twitch CDN 下载
```

## 三、匿名 GraphQL 请求形式（参考 Streamlink 当前实现）

`POST https://gql.twitch.tv/gql`，JSON body：

```json
{
  "operationName": "PlaybackAccessToken",
  "variables": {
    "isLive": true,
    "login": "solarbacca",
    "isVod": false,
    "vodID": "",
    "playerType": "embed",
    "platform": "site"
  },
  "extensions": {
    "persistedQuery": {
      "version": 1,
      "sha256Hash": "ed230aa1e33e07eebb8928504583da78a5173989fadfb1ac94be06a04f3cdbe9"
    }
  }
}
```

请求头参考：`Client-ID`（Twitch 网页客户端的内部标识）、`User-Agent`、`Content-Type: application/json`。本次测试的 Referer/Origin 为 `https://player.twitch.tv/`，匿名测试成功。此值及 persisted-query 哈希并非 Twitch 向第三方应用承诺的稳定 API；必须为接口变化设计独立错误分类与维护策略。

响应解析路径：`data.streamPlaybackAccessToken.signature` 与 `data.streamPlaybackAccessToken.value`；值为空不能简单当作 `API 失败`，应结合频道状态区分离线与请求错误。

频道信息可参考两个查询：
- `ChannelShell`，hash `fea4573a7bf2644f5b3f2cbbdcbee0d17312e48d2e55f080589d053aad353f11`，variables `{"login":"solarbacca"}`
- `StreamMetadata`，hash `b57f9b910f8cd1a4659d894fe7550ccc81ec9052c01e438b290fd66a040b9b93`，variables `{"channelLogin":"solarbacca","includeIsDJ":true}`

授权用户状态、订阅限制、Client-Integrity challenge 不能靠匿名成功样本证明永远不需要处理。Streamlink 的当前策略是先尝试匿名请求，仅在失败时尝试需要 Chromium 的 Client-Integrity 路径；栖流首版应明确报错并保留打开官方网站的回退，不宜承诺纯 Rust 能应对所有账户/频道挑战。

## 四、Usher 地址及本次真实画质样本

地址：`https://usher.ttvnw.net/api/v2/channel/hls/{lowercase_login}.m3u8`。

参数建议参考 Streamlink：`platform=web`、`allow_source=true`、`allow_audio_only=true`、`playlist_include_framerate=true`、`multigroup_video=true`、`supported_codecs=h264`、`p=<cache-buster>`、`sig=<URL-encoded signature>`、`token=<URL-encoded value>`。使用 `url::Url::query_pairs_mut()` 编码，不能手动拼未转义 token。

2026-10-15 对 solarbacca 得到 6 条 variant：
- 1920×1080 / 60 fps / H.264 + AAC / BANDWIDTH 6915921
- 1280×720 / 60 fps / H.264 + AAC / BANDWIDTH 3422999
- 852×480 / 30 fps / H.264 + AAC / BANDWIDTH 1427999
- 640×360 / 30 fps / H.264 + AAC / BANDWIDTH 630000
- 284×160 / 30 fps / H.264 + AAC / BANDWIDTH 230000
- 音频独立流 / AAC / BANDWIDTH 160000

顺序可能变化；不能假定第一条就是最佳画质。应只解析 `#EXT-X-STREAM-INF` 关联的下一条相对或绝对 URI，并用 `Url::join`。过滤未知/不支持的 codec；允许音频独立模式，但不要默认选音频。输出 `1080P60` 等明确标签。

项目 `StreamQualityOption.value: u32` 和前端的 `requestedQualityValue` 可复用，但建议使用 **稳定的档位键**（例如按分辨率和帧率编码）而不是 master playlist 中可能波动的 BANDWIDTH。保留 `0` 表示自动/最高可用。首次最小可用版本可以选择固定变体；真正 ABR 可以后续切换为交付 master 清单并记录 HLS.js 实际档位。Twitch 没有证实可选择多个不同 CDN「线路」，首版 `line_count = 1`，不把 6 个画质误称为 6 条线路。

## 五、跨平台 Origin / HLS.js 的重要发现

本次探测的 CDN：
- variant playlist 主机：`*.playlist.ttvnw.net`
- segment 主机：`*.hls.ttvnw.net`，如 `*.cloudfront.hls.ttvnw.net`
- 1080p60 的 segment 为 `.ts`

在同一条有效的 variant 播放列表上：
- 无 Origin：HTTP 200 / `Access-Control-Allow-Origin: *`
- `Origin: tauri://localhost`：HTTP 200 / `*`
- `Origin: https://player.twitch.tv`：HTTP 200 / `*`
- `Origin: http://localhost:1420`：HTTP 200 / `*`
- `Origin: http://tauri.localhost`：HTTP 403 / `*`（重复验证）
- `Origin: https://tauri.localhost`：HTTP 403 / `*`
- `Origin: null`：HTTP 403 / `*`

两种 Tauri Origin 的 segment Range 请求均成功返回 206。注意：这些是 Python 发出的模拟请求，并不是 Windows/Android WebView 端到端测试。

Tauri 官方资料表明打包应用在 macOS 使用 `tauri://localhost`，Windows/Android 默认使用 `http://tauri.localhost`。因此不要直接假设从 Rust 返回播放列表 URL 后，所有平台都能通过 HLS.js 获得清单。

**建议方案**：先验证实际 WebView。若复现 403，只为 Twitch 实现 HLS.js `pLoader` + Rust `fetch_twitch_playlist`：Rust 负责受限 GET HLS 文本播放列表；HLS.js 负责解码清单并原生 GET 视频分片。如此避免整条媒体代理造成的内存/吞吐瓶颈。若将来分片也被拒绝，再设计完整流式代理；不要默认强制把所有视频字节经 Rust IPC 中转。

Rust 代理必须检查 HTTPS、精确允许列表/可信域名后缀、重定向目标与内容类型，设置响应大小及超时上限；不得成为任意 URL 下载器、SSRF 代理。播放列表要允许刷新且不得被错误地长期缓存。

## 六、栖流实际需要修改的文件（未来实施阶段）

- `src-tauri/src/twitch.rs`：新增 `TwitchClient`、URL/频道验证、查询、令牌、Usher、HLS master 解析、`is_live`、`resolve` 与测试。
- `src-tauri/src/lib.rs`：`mod twitch`、`LiveClients` 中加入 TwitchClient、`SourcePlatform::Twitch`、`source_platform` 识别、resolve/get_live_status 分发、如需则安全 playlist IPC 命令。
- `src/source-library.ts`：`LivePlatform` 增加 `twitch`、直播链接规范化（`twitch.tv` / `www.twitch.tv` / `m.twitch.tv`），收藏与候选匹配；严格阻止二级域名仿冒、附加端口、非法频道名。
- `src/source-library.test.ts`：规范化、拒绝不可信域名、恢复收藏持久化。
- `src/playback-supervisor.ts`：平台名称；若必要，针对 Twitch 的 `pLoader`；选画质/重连处理；可选实时 ABR。
- `src/playback-supervisor.test.ts`：离线、无权访问、网络失败、HLS 失败恢复、画质档位行为。
- `src-tauri/tauri.conf.json`：**最小化**扩展 CSP 的 `connect-src`、`media-src`、`img-src`，例如对 `*.ttvnw.net` 的媒体域；头像可能来自 `static-cdn.jtvnw.net`。不要全开放 `*`。
- `README.md`、`CHANGELOG.md`、现有平台支持提示：更新支持平台与已知限制。

可复用现有 `src-tauri/src/stream.rs::LiveStream` 的数据模型，无需更换播放器。

## 七、建议实现切分与验收条件

1. **Rust 单元层**：URL 边界、GraphQL 响应（离线/封禁/错误/有效）、含逗号及引号的 M3U8 属性解析、相对地址、codec 过滤、画质排序、签名编码、令牌脱敏。使用静态最小化 fixture，无须假冒媒体资源。
2. **Rust 实网探针**：标记 `#[ignore]`、使用环境变量指定频道（例如 `QILIU_TWITCH_PROBE`），只记录 HTTP 状态和画质摘要，不记录签名。分别测正在直播和离线频道。
3. **集成层**：实现统一 `LiveStream`、状态与收藏。在线返回 `format=hls` 和有效线路；离线返回 `url=None`，接口失败不得显示为离线。
4. **HLS.js 播放**：真实频道首次出画、有声、选择 720p/1080p、刷新清单、断线恢复，检查短缓冲与广告时段。检测实际码率与画面帧率。
5. **平台**：macOS 打包、Windows WebView2、Android WebView 分别验收首次和持续播放；尤其排查 `http://tauri.localhost` 的 playlist 403。
6. **维护与回退**：GraphQL persisted-query hash 变化、Client-Integrity 要求、403/401、地区或订阅限制，提供明确错误与打开官网入口，不静默降为「未开播」。

## 八、优先级与边界

- P0：无认证的公开 Twitch 频道直播，H.264/HLS，手动画质，连接/重连，macOS。
- P0 跨平台发布前：Windows/Android playlist Origin 403 解决并分别实测。
- P1：自动 ABR、封面和头像、离线等待、更多房间及长时运行验证。
- P2：授权用户流、VOD/Clip、额外 codec（H.265/AV1 需检测各平台解码能力）；不要绕过订阅权限、地区限制或广告机制。

该设计依赖非公开的网页 GraphQL 与 Usher 机制，稳定性不可与 Twitch 官方 API 等同。Twitch Helix 的 Get Streams 可用于合规的直播状态查询，但要求应用/用户 access token，且它不提供可直接播放的 HLS URL。Twitch 官方嵌入式播放器是另一条合规备选路径，但它要求合规的 `parent` 域名、具有嵌入要求，与栖流当前 `frame-src 'none'` 和自定义桌面应用 origin 不直接兼容。

## 九、参考

- Streamlink Twitch 实现： https://github.com/streamlink/streamlink/blob/master/src/streamlink/plugins/twitch.py
- Streamlink Twitch 客户端完整性说明： https://streamlink.github.io/cli/plugins/twitch.html
- Twitch Helix API： https://dev.twitch.tv/docs/api/reference
- Twitch 官方 Embed： https://dev.twitch.tv/docs/embed/video-and-clips/
- Tauri 自定义协议 Origin： https://docs.rs/tauri/latest/tauri/struct.Builder.html 及 https://v2.tauri.app/reference/javascript/api/namespacewebview/
- HLS.js 解析器： https://github.com/video-dev/hls.js
