# 虎牙直播：单播放器与可续期 HLS 会话架构（ADR）

日期：2026-10-15
状态：**首版动态续签 HLS 已接入默认虎牙直播路径；原生长播和跨端验收未完成**
项目：栖流 Qiliu v1.5.0
关联文档：
- `docs/huya-stream-source-system.md`
- `docs/live-playback-fix-2026-09-28.md`
- `docs/huya-flv-handoff-2026-10-15.md`

## 0. 架构决策

首选：**一个 HTMLVideoElement + 一个 Hls.js 实例 + 一个 MediaSource 生命周期，使用可续签的逻辑直播会话作为取流抽象**。

- 网络层的播放地址可以更新，播放器不得因此销毁或更换。
- 保留原 HTTP-FLV 为兼容/回滚路径，不再把双播放器渐变视为核心解决方案。
- 用户界面的「播放器连接」标识 `session_id`（频道 + CDN + 画质），而不是一次性 CDN URL。
- “无黑屏”是独立于“零跳帧”的视觉验收指标；链路中断或服务端内容不连续时不能保证物理意义上的零丢帧。
- 不依赖 Streamlink、ffmpeg 或独立本地代理进程。

## 1. 现场证据与可行性边界

2026-10-15 真实房间 `691406`：
- HS HTTP-FLV 连接：120.4 秒正常 EOF，累计 45,913,728 bytes，最大数据块间隔 384 ms；流持续可用但连接有有限时长。
- 同一条带签名的 HS HLS 媒体列表：0–130 秒每 10 秒刷新均为 200，140 秒返回 403；在测试时 `wsTime` 距过期尚约 24 小时。
- HLS 列表通常是 1–3 个分片（约 4–12 秒），刷新序号不断向前。
- 从同一房间/HS CDN 重新取得完整的新签名后立即返回 200。
- **新旧不同签名**对应的 HLS 清单可以有相同 `#EXT-X-MEDIA-SEQUENCE`（实例：1792072072）和相同媒体分片路径，表明新签名不必重置内容时间轴。
- `Origin: tauri://localhost` 和模拟 `http://tauri.localhost` 在这次请求中都收到 HTTP 200。

局限：这不等于已完成 Hls.js/WebKit 不间断长播放；也不证明全部 CDN、码率、广告事件期间都共享相同时间轴。分片内部 PTS/DTS、实际持续播放与 `#EXT-X-DISCONTINUITY(-SEQUENCE)` 仍需要验证。

## 2. 严格拆分三个生命周期

```text
PlaybackSession (频道 + CDN + 画质)         —— 可持续数小时，用户切源才结束
  ├── Lease (临时 signed media URL)        —— 预计几十秒至约两分钟更新
  │     ├── generation / acquired_at / expires_at_observed
  │     └── refresh gate / 403 recovery
  ├── Playlist timeline (media sequence)   —— 不因换签名重置
  │     ├── media sequence / discontinuity sequence
  │     ├── chunk URI / sequence identity
  │     └── timestamp/codec continuity checks
  └── Video + Hls.js + MediaSource         —— 换令牌不能重建
```

强制约束：
1. `lease_renewed` 不触发 `video.pause/load`、`hls.destroy()`、`video.src` 变更、`new Hls()` 或 `new MediaSource()`。
2. 新令牌失败时旧有效令牌保持使用，按指数退避继续获取；不要把续签瞬态故障映射成主播离线。
3. 新旧播放列表的 `MEDIA-SEQUENCE` 必须可衔接；不能用新列表覆盖逻辑时钟来实现“假连续”。
4. 同一会话首选同 CDN、固定 codec 与画质；失败再按明确规则迁移，不能签名续租时悄悄换 CDN。

## 3. 推荐模块（不大改现有公共 LiveStream）

### Rust

`src-tauri/src/huya.rs`
- 保留 `resolve` 的现有 FLV 逻辑供回退；
- 增加选择 HLS 线路/码率的显式方法，不要用 `extract_live_lines` 中“FLV 优先且覆盖 HLS”的候选集合；
- 同房间 `profileRoom` 的 `sHlsUrl` / `sHlsAntiCode` 配合已有 `build_play_url` 逻辑签名；
- 对新地址合法性、域名、用户输入严格校验。

`src-tauri/src/huya_hls_session.rs`（新增）
- 管理 `HuyaHlsSession`：`source, cdn_type, quality, current_lease, pending_renewal, last_playlist_sequence`。
- 使用 `tokio::sync::Mutex` 或共享 in-flight future 保证并发多次 HLS playlist 请求时只续签一次。
- `get_playlist(session_id)`：读取当前签名 URL 的真实 HLS 列表（HTTP 文本，不转发媒体大流），设置 `Cache-Control: no-store` 等价请求语义；大小上限、超时、白名单及每条响应 URI 的安全校验。
- `renew(session_id)`：后台获得新的签名 URL，**先预检新列表和序号关系**，成功后原子发布新 Lease；旧 Lease 在安全回退窗保留。
- `start_huya_hls_session(source, quality, line)` / `read_huya_hls_playlist(session_id)` / `close_huya_hls_session(session_id)` 分别为 Tauri 命令（也可通过受限内部协议提供播放列表）。
- 防泄露：日志永不输出完整带签名 URL；只记录 CDN、序号、错误码、签名代数及延迟；session ID 用不可预测随机值并绑定生命周期，禁任意 URL 代理。
- Rust 前置续租默认参数可试 `renew_after=60s`（小于当前一次观察到的 140s 403，**不是签名有效期保证**）；如 401/403 或过期立即有限次刷新；远端实际策略可变。

### TypeScript

`src/huya-hls-playlist-loader.ts`（新增）
- 自定义 Hls.js `pLoader`，对 `manifest/level` 类 HLS 文本请求调用 Rust 会话接口，保持稳定的**逻辑 playlist URL**；返回真实文本和可用于解析相对分片地址的原始有效 CDN 基地址（或将片段 URL 解析为绝对地址）。
- 不把 TS/fMP4 媒体分片经过 Rust IPC；分片保持默认 HLS.js 网络加载器。
- 严格实现 Hls.js Loader 的 `load/abort/destroy/stats/context` 语义，携带当前播放 generation，取消的回调不能污染下一个频道。
- 使用类似 `src/hls-playlist-policy.ts` 的 `FetchLoader` 与 `fetchSetup` 可作为更小的 PoC：在签名过期前重写 playlist 的 Request URL，并通过原有的 `response.url` 解决片段 URI 基址。这一方案无需完整 Rust 文本代理，优先评估它能否稳定处理网络 403、跨平台 CORS 与缓存。不能只在载入初始 HLS 时替换一次 URL。
- 对 `pLoader` 维护一个单一的 Hls.js 实例；`LEVEL_LOADED` 的序号持续增长视为正常，`403` 先触发租约续签，不立即调用 `playUrl()`。

`src/playback-supervisor.ts`
- 建立 Huya HLS 会话后只调一次 `attachHls`；再把后续 playlist 续签全部委托给独立 transport 层；
- `PlaybackSupervisor` 只对不可恢复的媒体解码、长时间无有效分片、离线等做**会话级恢复**；
- 双视频 FLV `prepareHuyaContinuation` 保留作兼容 fallback，一旦 Huya HLS 稳定，则从默认路径和正常续签逻辑中移除；
- 播放器统计分开计数 `tokenRenewalCount`、`playlistReloads`、`mediaRestarts`、`stallCount`、`droppedFrames`、`blackFrames`，避免把“正常的签名更新”记为重连。

## 4. 稳定会话的取流时序

```text
START
  -> resolve_huya_hls(source, line, quality)
  -> initialize Hls.js only once
  -> on each HLS playlist load:
       old lease healthy?  -> fetch playlist (no-store)
       renewal due?       -> obtain new signed URL in parallel
       new playlist ready?-> compare sequence/codec/discontinuity
       accepted?          -> atomically swap lease (no video change)
       old URL 403?       -> renew immediately, retry a bounded number of times
       no valid playlist? -> keep existing video surface, show buffering, retry
  -> on user stop/source change: destroy whole session explicitly
```

接缝校验：如果上一次列表媒体序号是 S、长度 N，新的列表媒体序号是 T，允许正常直播滑窗 `T >= S` 且最新片段可连续衔接；如果相差超过本地最大缓冲能覆盖的窗口，应用可以追赶实时边缘，但应记录跳帧而不是宣称无缝。若 `DISCONTINUITY-SEQUENCE` 改变，交给 Hls.js 正常解码器重配置，不应假定 PTS 可以直接拼接。

## 5. 失败与回退状态机

```text
ACTIVE
  ├── near expiry -> RENEWING ──success+timeline OK──> ACTIVE
  │                       └──failure──> ACTIVE(old lease) / RETRYING
  ├── playlist 401/403 -> RENEW_NOW -> REFRESH_PLAYLIST -> ACTIVE
  ├── fragment error -> Hls.js normal retry -> 复核签名 / CDN
  ├── sequencer gap -> RECOVERING_LIVE_EDGE (same Hls instance preferred)
  ├── long stall / unsupported codec -> FALLBACK_TO_FLV (requires player rebuild once)
  └── offline / source changed -> CLOSE
```

关键：续签失败不允许误触发 `restartCurrentSource`。同一份 HLS 直播会话不能在用户无感知的情况下反复调用 `video.load()`。

## 6. 验收门槛

A. **真实网络**（多次、不同房间/线路）：签名跨越一次旧 URL 403 时，签名更新后的 `MEDIA-SEQUENCE` 仍单调；分片依然可 GET；无重复老片段；检查 TS 的 DTS/PTS 单调性与 discontinuity 信息。

B. **Hls.js 集成**：至少 30 分钟连续观看（含 >10 次续签），`new Hls()`、`hls.destroy()`、`video.load()` 各只在开启/关闭会话阶段运行；续签造成的播放器重建次数必须为 0。

C. **用户可见**：至少跨越 10 次服务端有效期边界；用 `requestVideoFrameCallback` 和屏幕采样记录无周期性黑帧，无重复片段；评估有无冻结、声音中断及帧跳跃。网络实际不可用时不承诺绝对零中断。

D. **跨平台**：macOS WKWebView、Windows WebView2、Android WebView 分别跑实际播放。模拟 Origin 的 HTTP 200 不等于这些端的媒体可用。

E. **失效模拟**：令牌签发失败、签名 403、同一签名 2 个并发续签、旧会话销毁后迟到响应、断网恢复、不同码率/线路、更换频道、正在重签时暂停、CDN 播放列表序号回退等都需要自动化回归。

## 7. 实施顺序（按可回滚增量）

- [x] 实测旧 FLV 接口约 120 秒正常 EOF
- [x] 旧 HLS URL 约 140 秒失效，并证实新签名 URL 可再用
- [x] 对照新旧签名的 `MEDIA-SEQUENCE` 和共同分片
- [x] **PoC 1**：Rust 显式解析并验证 HLS 清单及首个 TS 分片，保留原 FLV resolver
- [x] **PoC 2**：独立 `pLoader` + 单视频 Hls.js 动态续签、HTTP 403 原地恢复与播放列表禁用缓存（代码与单元测试通过）
- [ ] **PoC 3**：在真实 WKWebView 连续 30 分钟，核验签名过期、长播、实际分片播放及跨端 CORS（目前完成 HTTP 实网探针，尚未完成 WebView 实机）
- [ ] **产品化**：引入 SessionLease 状态机、错误分类、指标采样及灰度/回滚开关
- [ ] **发布**：三平台验收通过后才将可续签 HLS 设为虎牙默认；FLV 降为兼容路径

## 8. 为什么不优先写原生 FLV 拼接器

另一条同样根本的路线是让 Rust 持续拉取有限寿命 FLV，再用自己的 demux/transmux 维护连续 PTS/DTS，把每次 HTTP 连接拼入**同一个** MediaSource。其难点在于 FLV tag header 重置、H.264 IDR/参数集、AAC AudioSpecificConfig、时间戳重映射、音视频同步、MSE SourceBuffer 管理和平台 WebView 差异。

HLS 已有可续的 `MEDIA-SEQUENCE` 和 Hls.js 成熟的清单机制；因此先验证 HLS 常驻播放器，是更少自造底层音视频基础设施的方案。若这条路线无法证明稳定，再考虑单 MSE FLV 拼接，并以明确实验数据作决策。

## 9. 外部参考

- Hls.js PlaylistLoader： https://github.com/video-dev/hls.js/blob/master/src/loader/playlist-loader.ts
- Hls.js Loader 类型： https://github.com/video-dev/hls.js/blob/master/src/types/loader.ts
- mpegts.js： https://github.com/xqq/mpegts.js/blob/master/docs/livestream.md

**本 ADR 为根本性架构实施依据，不等于已经上线或已经证实完全消除闪烁。**


进度和现场证据：`docs/huya-dynamic-hls-implementation-2026-10-15.md`。
