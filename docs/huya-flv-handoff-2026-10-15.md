# 虎牙周期性画面闪烁：复查、取流对照与视觉交接修复

日期：2026-10-15
项目：栖流 Qiliu 1.5.0
状态：**源码修复 + 自动化测试通过；待 macOS 原生视觉验收**
验证房间：`https://www.huya.com/691406`（测试时正在直播，HS CDN）。

## 一、根因分层

- **传输层触发**：这次真实 HTTP-FLV 长连接在 120.4 秒正常 EOF。45913728 bytes，最大数据块间隔 384.39375 ms，>=1 秒间隔 0 次；排除了这次采样中持续网络断供，不能据此推断所有房间/CDN 都有固定时长。2026-09-27 旧采样的同类连接在 119.7 秒 EOF。
- **应用交接**：`src/playback-supervisor.ts` 在约 100 秒预热下一条 FLV 后，原实现立即 `oldVideo.pause()`、把新元素从 `opacity: 0` 改为可见并立刻销毁旧解码器。这没有留给 WKWebView 合成器完成新视频 surface 首次可见合成的交叠期；即使 `requestVideoFrameCallback` 返回和缓冲可用，也不等于 DOM 图层已完成可见帧的合成。
- **确定的样式缺陷**：最初的 `index.html` 使用只有 `id="live-video"`、没有 `class="live-video"` 的 video；在第一次续接时移除旧视频 id 后，旧元素不再匹配用于绝对定位的 `#live-video,.live-video`，会从全屏图层布局脱离。
- **机制上仍可能有内容跳帧**：新旧 HTTP-FLV 连接从不同实时刻开始读取。重建不同 MediaSource 会产生时间轴跳跃；视觉渐变能避免黑底露出，不等于能保证帧级连续或彻底消除内容的跳变。

## 二、已修改的代码

- `index.html`：初始 video 增加 `live-video` 类。
- `src/playback-supervisor.ts`：交接前为旧元素保留样式类；预热成功后旧元素保持播放，等待 380 ms 的画面渐变交接完成再暂停、销毁、移除；旧元素静音防止双重音频；在暂停、停止或切换源时提前清理尚待释放的旧播放器。保留预热 100 秒、按帧/缓冲校验、正常 EOF 处理等既有功能。
- `src/styles.css`：预热视频使用极低但非零透明度，降低 WKWebView 将完全透明的视频 surface 延迟合成的风险；正式交接时 260 ms 透明度渐变，并短时间保留旧图层。
- `src/playback-supervisor.test.ts`：断言旧画面不得提前暂停，380 ms 后才释放资源，暂停时必须立即完成旧图层清理。

## 三、测试

- `PATH=/opt/homebrew/bin:$PATH npm run test`：72 项全通过（本机默认 `/usr/local/bin/node` 是旧 20.9.0，当前 Vitest 4 下会出现误导性 hook timeout，使用已安装的 `/opt/homebrew/bin/node` 22.22.1）。
- `PATH=/opt/homebrew/bin:$PATH npm run build`：TypeScript/Vite 构建成功。
- `cargo test --manifest-path src-tauri/Cargo.toml --lib`：52 项通过，11 项真实网络测试按设计忽略。
- `SIMPLE_LIVE_HUYA_TEST_ROOM=691406 SIMPLE_LIVE_HUYA_MEASURE_SECONDS=132 cargo test --manifest-path src-tauri/Cargo.toml measures_current_huya_flv_chunk_gaps -- --ignored --nocapture`：120.4 秒 EOF，45.9 MB 数据，正常完成。

## 四、HLS 对照

2026-10-15 `691406` 房间提供 `sHlsUrl` 和 `sHlsAntiCode`，与 FLV 同一份 `profileRoom` 元数据中获取。按现有 Rust `build_play_url` 的签名算法构造 HLS 地址后：
- `Origin: tauri://localhost`、`Origin: http://tauri.localhost` 均返回 200；
- 获得短 HLS 媒体播放列表，最初约 1–3 个分片，刷新后增加到约 3 个，每次总计约 12 秒；
- 对**同一条签名 HLS URL**进行 140 秒实网刷新：0 到 130 秒共 14 次每隔 10 秒请求均返回 200，且媒体序号持续从 1792066471 增长到 1792066503；第 140 秒返回 HTTP 403；
- 该 URL 的签名字段 `wsTime` 在测试时显示还有约 86400 秒，并不保证实际 playlist 可用期；
- 再从房间接口取得新的签名并构造新 HLS URL，返回 HTTP 200，表明刷新签名能够立即重新获取播放列表；
- 403 的准确触发规则（会话寿命、服务器侧状态还是频控）尚未确定；必须结合 HLS.js 真机长播放确认。

**结论：只用静态 HLS URL 不足以消除周期性故障。** 新签名的 HLS 列表在本次现场能继续使用，因此后续如果希望用 HLS 消除 FLV 因 EOF 而重建 video 的问题，建议实现保留同一 HLS.js 实例的播放列表动态重签名：`hls.js pLoader` 请求 Rust 受限接口，Rust 查询下一条有效签名，并对清单衔接序号、分片签名等做验证；不能直接简单修改 m3u8 字符串，必须确保媒体时间戳/序号连续，防止旧分片回放、卡顿或 403。过去项目也记录了 HLS 短窗口/缓存问题，所以暂不把 HLS 设置为默认。

## 五、剩余工作 / 严格的“完全解决”验收

1. 在真实 macOS WKWebView 连续观看至少 6–10 分钟，跨越不少于 3 次预热交接；录像或按帧审看是否仍有任何黑帧、冻结或亮度突变。
2. 观察后台预热至渐变和旧图层销毁三阶段；排查 WebKit compositor 对 `opacity: 0.001` 和新视频 surface 的行为。记录交接前后 decoded frame count、播放头、媒体事件与缓冲。
3. 如果出现跳变但没有黑帧，进一步研究复用单个 `MediaSource` / 连续 FLV 拼接与时间戳重新映射；这需要正确处理第二次 FLV header、codec initialization、H.264 IDR、AAC 配置和 DTS/PTS 不连续，不能只把两个 HTTP 响应字节拼起来。
4. 若 HLS 长连接刷新和分片加载更稳定，可设计「HLS 优先，FLV 备用」试验开关，再对照 30 分钟真实直播的帧间隔、重连数与画面完整性，达到要求后再设为默认。
5. Windows/Android 仍需各平台视觉与实际媒体请求测试，不可用 macOS 的通过替代。

## 六、参考

- `docs/live-playback-fix-2026-09-28.md`
- `docs/huya-stream-source-system.md`
- https://github.com/xqq/mpegts.js/blob/master/docs/livestream.md
- https://github.com/xqq/mpegts.js/blob/master/docs/api.md

本次只修改上面列出的四个源文件及本文档。没有提交、发布或修改已有用户直播记录。
