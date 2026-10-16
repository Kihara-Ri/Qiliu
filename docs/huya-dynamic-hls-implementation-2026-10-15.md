# 虎牙动态续签 HLS：实现、实网验证和待验收项

日期：2026-10-15
项目：栖流 Qiliu v1.5.0
状态：**已接入默认虎牙 HLS 播放路径并通过单元测试和网络探针；原生 WebView 30 分钟长播尚待验收**
架构依据：`docs/huya-stable-hls-architecture.md`

## 1. 与旧版架构的根本区别

旧方案对虎牙 FLV 的 HTTP 连接结束约 120 秒进行新旧 video / mpegts.js 双播放器预热与交接，其视觉层需要合成两个 surface，可能出现闪烁。新默认方案改为：
```
Live room URL
  -> Rust HuyaClient.resolve_hls() 选择真实可用的 HLS CDN
  -> Hls.js 创建一次 + HTMLVideoElement + MediaSource
  -> HuyaPlaylistLoader (每次 manifest / level 加载)
       -> HuyaHlsLease (首次缓存原始签名，45 秒后自动刷新)
       -> Rust renew_huya_hls_lease (同 CDN、同码率签名)
       -> Rust 预检实际 HLS 清单和可下载 TS 分片
       -> FetchLoader(no-store, actual signed URL)
       -> hls.js 利用 response.url 恢复相对分片 CDN 路径
       -> 401/403: 同一 PlaylistLoader 强制续签并重试一次
  -> Hls.js 继续追加媒体片段，不因为普通续签销毁播放器
```
不会代理实际视频流，也不需要 FFmpeg/Streamlink/独立本地 HTTP 服务器。

## 2. 源码修改

- `src-tauri/src/huya.rs`：新增 `resolve_hls`、`renew_hls`、强制选 HLS 的线路提取，HLS CDN 清单/首个分片预检。正常 FLV `resolve` 保持原有行为。
- `src-tauri/src/lib.rs`：Tauri IPC 命令 `resolve_huya_hls_stream`、`renew_huya_hls_lease`。只允许虎牙域名传入。
- `src/huya-hls-playlist-loader.ts`：`HuyaHlsLease`，45 秒续签，复用并发 in-flight promise，前台列表加载不等待续签，失败保持旧有效 URL 并退避 10 秒，拒绝关闭后迟到响应；`pLoader` 实际网络请求使用 `cache: no-store`，遇 401/403 在同一 Loader 内重签并重试。
- `src/playback-supervisor.ts`：虎牙直播在 Hls.js 可用时优先调用新的 Rust HLS resolver；播放器只在首次连接建立，不因续签重建。累积故障重连后回退到原 HTTP-FLV；计入 HLS 会话签名续租次数。其他平台播放逻辑不变。
- `src/huya-hls-playlist-loader.test.ts`：租约、并发、失败回退、迟到响应、模拟 HTTP 403 换签名、Loader 关闭等用例。
- `src/playback-supervisor.test.ts`：正常虎牙直播选取新的 HLS IPC 路径，保留既有 FLV 回退测试。
- 早先修改但未提交的 `index.html` 和 `src/styles.css` 是独立的 FLV 双播放器过渡方案，本次没有删除，FLV fallback 仍会使用。

## 3. 关键实网发现

2026-10-15，直播房间 `691406`：
- 原 HTTP-FLV HS 长连接实测约 120.4 秒正常 EOF。
- 某个带签名 HLS URL 在 0~130 秒刷新 200，第 140 秒返回 403；新签名可以返回 200。
- 不同签名 HLS 列表可有相同媒体序号和分片路径，说明播放媒体时间轴不必跟着 URL 重建。
- **新增发现**：HLS 列表 HTTP 200 并不保证所有分片可获取。HS 线路某次最新分片 206，但最旧分片 404；同次 AL/TX 线路分片可下载。与请求 Origin 无明显关联。新 Rust `probe_hls_playlist` 因此要同时验证列表和第一个分片（范围读取），尝试切换备用 CDN。
- Rust 实网测试 `resolves_and_renews_current_live_hls` 成功验证新旧签名对应 HLS 清单及实际 TS 分片：HTTP 206，`Access-Control-Allow-Origin: *`。相应请求使用模拟 `http://tauri.localhost` Origin。
- **风险**：即使启动时首个片段可读，长时间运行仍可能遇到直播滑动窗口淘汰旧 TS 分片。因此必须在真实 Hls.js 播放中验证直播边缘策略和 fragment-level retry。

## 4. 可靠性策略

- 启动前确保 CDN 返回 HLS 媒体列表且首个 TS 分片可下载；不可用的 CDN 顺序重试，不将未验证的地址提交给视频解码器。
- 签名租约超过 45 秒时第一次播放列表刷新**非阻塞**触发后台续签；当前列表立即沿用旧 URL，续签完再原子发布；并发请求仅续签一次，失败退避 10 秒。
- 续签先完成新 HLS 列表与分片预检，确认成功才更新缓存的 signed URL；失败时暂时保留原 URL 并在下一次刷新重试。
- 如果旧签名返回 401/403，立即强制新签并重试该清单加载（不动 Hls.js 或 MediaSource）。
- 关闭/切源时拒绝应用迟到签名响应。
- 正常 HLS 网络恢复失败达阈值后回退现有 HTTP-FLV；两套通道互不混淆。
- 仅允许 HTTPS Huya 域名与 .m3u8 及必要签名字段；Rust 媒体预检禁用重定向、设置超时及清单大小上限。无任意 URL 代理，流量直接去 Huya CDN。
- 源日志不得输出完整签名地址。

## 5. 测试命令

在项目根目录执行：
```bash
PATH=/opt/homebrew/bin:$PATH npm run check
SIMPLE_LIVE_HUYA_TEST_ROOM=691406 \
  cargo test --manifest-path src-tauri/Cargo.toml \
  resolves_and_renews_current_live_hls -- --ignored --nocapture
```
实际联网测试必须选择正在直播的房间，且其 CDN 可能随时间变化。默认 Rust 单元测试会跳过所有联网探针。

## 6. 严格的完成定义（暂未全部通过）

- [x] Rust 获取 HLS 与独立续签接口
- [x] 强制 HLS 线路提取，不再默认取 FLV
- [x] Rust 预检播放列表和首个媒体分片，可换 CDN
- [x] hls.js 自定义 playlist loader，强制 URL 刷新但不重建 video
- [x] HTTP 403 同加载器续签、失败保留旧签名、并发复用及停止取消测试
- [x] 旧 HTTP-FLV 通路保留，出现持续失败可以回退
- [x] HLS 续签次数在播放状态中可观测
- [ ] macOS **原生 WKWebView** 连续长播至少 30 分钟，跨多个旧签名失效点验证没有重建或黑帧
- [ ] Windows WebView2 与 Android WebView 实机验证
- [ ] 监测真实直播掉帧、媒体清单滑窗、`#EXT-X-DISCONTINUITY`、PTS/DTS 与视频/音频恢复
- [ ] 在不同虎牙房间、不同 CDN、画质和网络切换条件下回归
- [ ] 发布安装包之前做视觉长播验收

## 7. 说明

这次是**可运行的代码实现**而不只是研究文档；但“原生 30 分钟没有黑帧”尚未验证，因此不能宣称彻底根治。生产默认改为 HLS，并保留旧 FLV 自动 fallback；尚未提交 Git、没有发布/替换安装包。
