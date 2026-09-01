# 栖流 Bilibili 与直播收藏技术说明

> 适用版本：栖流 1.0.0
> 更新日期：2026-09-01

## 1. 目标与边界

本系统在不改变“画面优先、同一时间只播放一路直播”的前提下提供三项能力：

1. 支持 `live.bilibili.com/<room_id>` 直播间；
2. 通过应用内二维码取得 Bilibili 登录状态，并按账号权限选择实际可用的最高画质；
3. 在本机持久化一份最多 50 项的虎牙 / Bilibili 收藏列表，并记住上次播放项。

不包含密码登录、导入浏览器 Cookie、弹幕、平台内容发现、关注关系同步或云端收藏。应用只展示服务端确认的画质，不把请求档位或档位列表当成实际播放档位。

## 2. 模块边界

| 文件 | 职责 |
| --- | --- |
| `src-tauri/src/bilibili_auth.rs` | 生成 / 轮询登录二维码、过滤 Cookie、验证账号、读写系统钥匙串。 |
| `src-tauri/src/bilibili.rs` | 校验链接、解析短房间号、获取房间与主播资料、请求播放线路、选择 H.264 FLV。 |
| `src-tauri/src/huya.rs` | 保留原有虎牙房间、签名、线路和回放解析。 |
| `src-tauri/src/stream.rs` | 定义两个平台共享的 `LiveStream` 返回结构。 |
| `src-tauri/src/lib.rs` | 识别链接平台，分发取流请求，并暴露不含 Cookie 的登录命令。 |
| `src/playback-supervisor.ts` | 接收统一直播结构，负责 MSE 播放、统计、卡顿监测、换线和退避。 |
| `src/source-library.ts` | 规范化链接、旧数据迁移、收藏去重、激活、删除和元数据清洗。 |
| `src/main.ts` | 渲染收藏和账号状态，管理二维码轮询，并把主播资料写回收藏。 |

平台解析与播放器没有互相引用平台私有 JSON 字段。以后增加新平台时，只需返回 `LiveStream`，播放器恢复策略不用复制。

## 3. Bilibili 解析流程

### 3.1 链接校验

接受：

- `https://live.bilibili.com/5050`
- 省略协议的 `live.bilibili.com/5050`
- 含 `cid` 的官方活动播放器链接

房间号必须为 1–20 位数字。前端和 Rust 后端分别校验一次；后端不信任前端传入的主机名。

### 3.2 房间号与状态

先请求：

```text
GET https://api.live.bilibili.com/room/v1/Room/get_info?room_id=<输入房间号>
```

这个接口可直接接收页面中的数字房间号，响应同时包含正式 `room_id`、UID、标题、封面与直播状态，因而不再先串行调用 `room_init`。使用响应中的 `room_id` 作为正式房间号，避免短号和真实号不一致。`live_status` 的处理：

- `1`：直播；
- `2`：轮播内容，按回放状态展示，但仍用 FLV 长连接播放；
- 其他：未开播，保留主播资料并每 30 秒重新检查。

随后请求 `live_user/v1/Master/info` 取得主播名称和头像。主播资料请求失败不阻断播放，房间与播放线路请求失败才进入恢复流程。

### 3.3 播放地址

播放接口：

```text
GET https://api.live.bilibili.com/xlive/web-room/v2/index/getRoomPlayInfo
```

主要参数：

```text
room_id=<正式房间号>
protocol=0,1
format=0,1,2
codec=0
qn=10000
platform=web
ptype=8
no_playurl=0
```

最高画质连接采用两阶段请求：

1. 先以 `qn=0` 请求一次播放信息，读取所有 AVC codec 节点的 `accept_qn`；
2. 从中选择数值最大的档位，再用该 QN 请求一次新的播放信息；
3. 对返回候选先寻找 `current_qn == 请求 QN` 的线路；
4. 如果平台降档，则只保留服务端实际下发的同一 `current_qn` 线路；
5. 连续恢复阶段请求 `qn=400`，稳定 60 秒后重新探测最高档。

`accept_qn` 说明“这个响应声明有哪些档位”，请求参数说明“客户端想要哪个档位”，真正播放档位仍必须读取 codec 节点的 `current_qn`。

播放 URL 由三部分拼接：

```text
url_info.host + codec.base_url + url_info.extra
```

候选线路按照以下顺序排序：

1. 实际 QN 与请求 QN 一致；
2. 非 `mcdn` 线路优先；
3. `http_stream`；
4. `flv`；
5. `avc`（H.264）；
6. 恢复时按线路索引循环轮换。

只要响应中存在 `http_stream + flv`，候选集合就会移除同一 QN 的 HLS 清单。现场测试中 HLS 线路无法在 WKWebView 正常出画，不能让一次真正的 FLV 故障把恢复状态机轮换到已知不可用的传输格式；只有平台完全不返回 FLV 时才保留 HLS 作为兼容回退。

最终媒体 URL 必须是 HTTPS，且主机名属于 `bilivideo.com` 或 `bilibili.com`。这样即使平台 JSON 被异常内容污染，也不会让后端把任意第三方地址交给播放器。

## 4. 登录系统与凭据边界

### 4.1 为什么采用应用内扫码而不是读取浏览器

高画质与 Bilibili 账号权限有关，但直接读取 Safari / Chrome Cookie 会扩大权限范围、耦合浏览器加密存储，并可能拿到与本应用无关的数据。当前方案采用 Bilibili 网页二维码流程：

```text
GET https://passport.bilibili.com/x/passport-login/web/qrcode/generate
GET https://passport.bilibili.com/x/passport-login/web/qrcode/poll?qrcode_key=<短效 key>
```

二维码在 Rust 内编码为 SVG data URL。前端只接收图像和状态，不接收二维码原始 URL 或 `qrcode_key`。轮询状态：

- `86101`：尚未扫码；
- `86090`：已扫码，等待手机确认；
- `0`：登录成功；
- `86038`：二维码过期。

成功后 Rust 从响应头和官方跳转 URL 中提取 Cookie，并立即调用：

```text
GET https://api.bilibili.com/x/web-interface/nav
```

只有 `code=0`、`data.isLogin=true` 且 UID / 用户名完整时才接受登录结果。

### 4.2 Cookie 最小化与系统钥匙串

解析器只保留取流所需的登录字段：

```text
SESSDATA, bili_jct, DedeUserID, DedeUserID__ckMd5, sid
```

另从官方指纹接口取得当前客户端的 `buvid3` / `buvid4`。所有其他 `Set-Cookie`、路径、追踪字段和跳转参数都会被白名单过滤。

登录 Cookie 只存在于 Rust 内存与 macOS 系统钥匙串，钥匙串 service 为 `com.kiharari.simplelive.bilibili`，不会写入：

- 页面 JavaScript 或 DOM；
- `localStorage` 收藏数据；
- Tauri 命令返回值；
- 播放统计和日志。

前端最多看到 `authenticated`、用户名、头像、UID 与提示语。退出登录只删除本机钥匙串条目，不调用远程全局登出，避免影响浏览器和其他设备。

### 4.3 带登录状态的取流

房间资料与主播资料属于公开数据，不等待钥匙串，也不附带账号 Cookie。`getRoomPlayInfo` 会使用内存中已经就绪的登录 Cookie 与当前设备标识；若启动时钥匙串尚未读完，先用公开权限建立播放。后台登录验证完成后，只有当前实际 QN 低于原画档才重新解析一次，避免已经是原画时无意义地中断连接。登录成功或退出后，当前 Bilibili 收藏也会立即重新解析，旧的短效媒体 URL 不会继续冒充新权限下的画质。

### 4.4 为什么桌面端原先不能像网页秒开

以 `https://live.bilibili.com/22907643` 为同一测试目标，拆分建播瀑布后发现：

| 阶段 | 优化前实测 | 结论 |
| --- | ---: | --- |
| macOS 钥匙串登录 Cookie | 约 6–10 秒 | 最大阻塞项；系统安全服务是同步调用，且旧实现持有共享锁等待它返回。 |
| 房间、主播和两次画质请求 | 通常不到 1 秒 | 是必要网络请求，但不是主要长等待。 |
| CDN 响应头 | 约 0.12–2.05 秒 | 随线路与当时网络波动。 |
| 当前房间填充 768 KB | 约 68 毫秒 | 旧缓冲大小不是本次 6–10 秒问题的主因。 |

网页端的 Bilibili Cookie 已在浏览器进程和会话中可用，不需要跨到本应用的 macOS 钥匙串；网页也可以让房间数据与播放器初始化并行。独立 Tauri 应用为了不读取浏览器隐私数据，把登录凭据放在系统钥匙串，因此多了一次安全存储访问。这里的“网页可并行”是从可观察启动顺序做出的工程推断，不代表复刻了 Bilibili 网页内部实现。

修复包括：

1. 钥匙串读取移入阻塞线程池，且读取期间不再占用 Cookie 缓存互斥锁；
2. 播放解析只读取已经在内存中的账号状态，未就绪时立即走公开线路；
3. 删除可由 `Room/get_info` 覆盖的串行 `room_init` 请求；
4. 0.4.1 曾把 FLV stash 从 768 KB 调到 256 KB，以缩短首帧；0.4.3 保留虎牙 256 KiB，但把 Bilibili 提到 1 MiB，并启用独立解复用 Worker，兼顾启动与原画稳定性；
5. 登录后台就绪后按实际 QN 决定是否升级，已经是原画则不重启。

0.4.1 修改后同房间的公开建播元数据链路实测约 **1.16 秒**，其中媒体连接后填满 256 KB 约 **25 毫秒**。0.4.3 的 Bilibili 1 MiB stash 会根据当时码率增加约数秒启动缓冲，用更高的起始余量换取后续稳定。游客权限当时返回 `超清 · QN 250`；后台账号可用后会升级为账号实际允许的最高档。最终首帧还会受到 DNS、TLS、CDN、WebKit MSE 和设备解码状态影响，因此不能把单次测量承诺为固定值。

## 5. 实际遇到的画质问题

### 5.1 QN 是档位，不是码率

Bilibili 的常见直播 QN 对应关系如下：

| QN | 平台档位 |
| ---: | --- |
| 80 | 流畅 |
| 150 | 高清 |
| 250 | 超清 |
| 400 | 蓝光 |
| 10000 | 原画 |
| 20000 | 4K |
| 30000 | 杜比 |

因此“蓝光 · QN 400”不是 400 kbps，也不固定等于某个分辨率。它是平台用于请求和确认画质层级的枚举值；同一个 QN 的实际分辨率与编码码率可能随主播推流、转码策略和时段变化。应用里的“实时码率”和“分辨率”来自播放器实际收到的数据，更适合判断眼下播放的真实质量。

本应用先请求最高可用档；连续恢复达到第三次时会请求 QN 400。0.4.3 还会记录去重后的真实欠缓冲：90 秒内 3 次欠缓冲，或一次欠缓冲同时满足“缓存不高于 3 秒、累计至少 1,500 帧、掉帧率不低于 8%”时，保持当前 CDN 并改请求 QN 400。该稳定画质只在当前会话内保持；用户重新选择直播间后仍从最高档开始。因此看到 QN 400 可能是平台主动降档，也可能是播放器为了连续观看触发了稳定画质。

### 5.2 请求原画但实际只有超清

测试房间 `https://live.bilibili.com/5050` 时，请求 `qn=10000`，响应仍列出 `accept_qn=[10000,400,250]`，但 codec 节点明确返回 `current_qn=250`，实际视频为 1280×720。直接把请求值或 `accept_qn` 第一项显示为“原画 / 1080P”会虚报画质。

根因不是播放器解码分辨率受限，而是未登录请求虽然在 `accept_qn` 中列出更高档，服务端却在第二次请求中继续下发 `current_qn=250`。因此只把参数改成 `qn=10000` 并不能获得原画。

解决方式：

- 提供显式二维码登录，让请求带上账号权限；
- 每次最高画质连接先发现可选 QN，再请求最大值；
- 只用 `current_qn` 判断当前档位；
- 使用响应 `g_qn_desc` 中与 `current_qn` 匹配的描述；
- 界面显示如 `超清 · QN 250`；
- 线路集合只保留同一个实际 QN，避免换 CDN 时暗中换画质；
- 登录成功后重建当前播放器连接。

即使已登录，平台仍可能因房间、账号等级、地区、直播内容或接口策略降档；应用不能绕过平台权限。此时界面继续展示服务端实际 QN，而不是保证固定 1080P / 4K。当前播放器也只选择 H.264 FLV，不选择浏览器支持不一致的 HEVC / AV1 档位。

### 5.3 原画频繁缓冲：网络正常但 WebKit 供帧落后

2026-09-01 使用已登录账号和房间 `5050` 复现原画 `QN 10000`。旧配置没有触发重连或换线，但缓存从 6.4 秒依次降到 3.3、0.7、0.2 秒，累计掉帧从约 34% 升到 42%，与用户看到的短暂“正在缓冲”一致。

为区分 CDN 与播放器，使用独立 Rust 流读取器并行测量同一响应的前两条 FLV 线路 120 秒；测试不打印带签名 URL：

| 线路 | 120 秒接收量 | 最大数据块间隔 | 大于等于 1 秒的间隔 | 是否 EOF |
| --- | ---: | ---: | ---: | --- |
| `ov-gotcha07` 线路 1 | 37,002,980 B | 0.999 秒 | 0 | 否 |
| `ov-gotcha07` 线路 2 | 36,995,072 B | 0.804 秒 | 0 | 否 |

网络层连续送达而播放器缓存仍耗尽，因此根因不在“该 CDN 每隔一段时间断流”，而在 WKWebView 内的 FLV 解复用、MSE append 与原画解码链路没有持续跟上。mpegts.js 文档也说明 stash 用来吸收网络抖动，Worker 可以把解复用移出页面线程。0.4.3 的处理是：

1. 仅对 Bilibili 直播启用 mpegts.js Worker，虎牙继续使用已经验证的主线程续接路径；
2. Bilibili `stashInitialSize` 提高到 1 MiB，虎牙保持 256 KiB；
3. 自动清理继续使用 180/120 秒，避免旧版 60/15 秒在播放点附近频繁删除 SourceBuffer；
4. 对 `waiting` 与 `stalled` 做 4 秒去重，只有缓存不高于 0.5 秒才记录为真实欠缓冲；
5. 原画在 90 秒内发生 3 次欠缓冲，或欠缓冲同时出现高掉帧和低缓存时，不换 CDN，改请求 QN 400；设置面板如实显示稳定画质、缓冲、降档和换线次数。

现场对照中，Worker + 1 MiB 后 83 秒缓存从 8.2 秒增长到 19.1 秒、掉帧约 0.32%；到 221 秒时缓存回落到 2.5 秒、掉帧约 8.95%，说明前端处理压力显著下降但原画仍可能在长播中逼近设备极限，因此必须保留基于真实播放指标的会话内降档。

最终 0.4.3 隔离应用又进行了 344 秒长播：保持 `原画 · QN 10000`、1920×1080、约 60 fps，采样时缓存 7.9 秒、掉帧 0、故障重连 0、换线 0。由于这轮没有满足降档条件，应用没有无故降低画质；单独的真实解析测试确认稳定请求会得到 `蓝光 · QN 400`。这是一台机器、一个房间和当时网络的现场证据，不等同于所有环境的固定保证。

也测试了接口同时返回的 HLS 备用：TS 清单只有 2 个分片、总计约 8.3 秒；在同一 WKWebView 中 43 秒仍没有出画。它不能作为此次修复的默认替代，最终仍保留 Pure Live 同方向的 H.264 FLV，并在当前 Tauri/WebKit 能力边界内增加 Worker、缓冲和诚实的稳定画质策略。

## 6. 链接粘贴、清洗与收藏持久化

输入框同时处理 WebView 原生 `paste` 事件与 `⌘V` / `Ctrl+V` 键盘事件。后者通过 Tauri 剪贴板插件读取纯文本，因此即使 WebKit 没有把系统剪贴板内容交给输入框，也能完成粘贴。应用能力文件只开放 `allow-read-text` 和 `allow-write-text`，不开放图像、文件或任意系统能力。

粘贴后先移除零宽字符，再从整段分享文案中提取第一个虎牙或 Bilibili URL，去除中文括号与句末标点，最后通过 URL 解析器清除 query 和 fragment，并重建为：

```text
https://www.huya.com/<room_id>
https://live.bilibili.com/<room_id>
```

不支持的域名、用户名密码式 URL、非法房间号不会进入收藏。输入框和设置面板允许文字选择；`⌘C` / `Ctrl+C` 只把当前选中文本写入剪贴板。

新存储键：

```text
simple-live.source-library.v1
```

结构：

```json
{
  "version": 1,
  "activeId": "bilibili:5050",
  "favorites": [
    {
      "id": "bilibili:5050",
      "source": "https://live.bilibili.com/5050",
      "platform": "bilibili",
      "platformLabel": "Bilibili",
      "roomId": "5050",
      "anchor": "主播名",
      "title": "直播标题",
      "avatarUrl": "https://...",
      "addedAt": 0,
      "lastPlayedAt": 0
    }
  ]
}
```

ID 由平台与规范化房间号组成，用于去重。再次添加同一链接只会更新时间并将它移到列表首位。收藏只存可重新取得的页面链接和展示元数据，不保存短效播放 URL、平台签名或 Cookie。

### 6.1 旧版本迁移

如果新键不存在或内容损坏，读取旧键：

```text
simple-live.huya-source.v1
```

旧链接通过新的双平台规范化函数校验后变成第一条收藏，随后立即写入新结构。旧键暂时保留，便于异常情况下回退，不会覆盖新列表。

### 6.2 损坏数据处理

读取 JSON 时逐项重新规范化：

- 删除非虎牙 / Bilibili 项；
- 删除重复 ID；
- 限制为 50 项；
- 文本元数据截断为 1024 字符；
- `activeId` 不存在时回退到第一项；
- 完全无法恢复时返回空列表并打开设置面板。

## 7. UI 与播放连续性

收藏列表位于原有常驻设置面板内。面板仍始终存在于固定合成层，收起只做 `translate3d`，不会插入 / 删除 DOM 或改变播放器宽度，因此新增列表不会重新引入“黑块挤压画面”的旧问题。

切换收藏时：

1. 立即持久化新的 `activeId`；
2. 清除旧播放器实例、计时器和恢复状态；
3. 请求新平台的房间与短效线路；
4. 新流开始后把主播名称、标题和头像写回当前收藏。

删除非当前项不影响播放。删除当前项时自动选择列表首项；列表为空则停止播放器并展开设置面板。

账号区与二维码放在同一个常驻设置面板中。二维码只在账号区内部出现，不改变播放器宽度，也不创建新窗口。二维码轮询严格串行，每次新二维码会使上一轮轮询失效，避免旧请求晚到后覆盖新状态。

## 8. 验证

`npm run check` 包含：

- 7 个收藏库测试：双平台规范化、分享文案和零宽字符清洗、非法链接、旧配置迁移、去重与激活、删除回退、元数据持久化；
- 8 个播放恢复策略测试：同线路刷新、短时间重复失败换线、Bilibili 欠缓冲去重和稳定画质判定；
- Rust 测试：平台路由、Bilibili 链接解析、最高 QN 发现、服务器降档、同 QN 线路筛选、Cookie 白名单、账号数据脱敏、二维码编码、媒体域名白名单及全部虎牙用例；
- TypeScript 严格检查和 Vite 生产构建。

另有默认忽略的真实网络测试，会连接当前直播中的公开 Bilibili 房间、验证最终响应为 `video/x-flv`、测量 FLV 数据块间隔并检查 HLS 清单时长。可以通过 `SIMPLE_LIVE_BILIBILI_TEST_ROOM` 指定当前开播房间。这些测试依赖第三方房间状态，不放入离线默认测试。

真实登录后的最高档验证必须由用户扫描短效二维码；自动化测试不会读取浏览器登录状态，也不会内置账号凭据。验证时以设置面板显示的 `current_qn`、分辨率和实际 FLV 响应为准。

0.4.1 的隔离应用界面回归使用整段分享文案 `https://live.bilibili.com/22907643?...`：粘贴后输入框自动规范化为无参数链接，提交后约 2.0 秒进入 `LIVE`，已授权账号实际显示 `原画 · QN 10000`。该秒数是一次本机实测，不作为固定性能承诺。

0.4.3 的隔离回归使用房间 `5050` 与已授权账号，验证原画先起播、线路标签明确显示 `FLV`，并持续观察缓存、掉帧、真实欠缓冲及自动 QN 400 策略。这里的长播采样用于验证状态机，不代表对所有机器和网络承诺相同数值。

## 9. 后续扩展点

- 可将收藏从 `localStorage` 迁移到 Tauri Store；迁移时保留当前 JSON 版本号；
- 可增加用户手动选择“最高 / 蓝光 / 超清”，但必须继续展示服务端实际 QN；
- 可为每个平台增加独立的失败指标，但恢复状态机继续保持共享；
- 可在收藏中加入手工备注或排序字段，不应把它扩展为平台内容推荐页。

## 10. 参考

- [Pure Live](https://github.com/liuchuancong/pure_live)
- [Pure Live 的 Bilibili 解析实现](https://github.com/liuchuancong/pure_live/blob/master/lib/core/site/bilibili/bilibili_site.dart)
- [Pure Live 的 Bilibili 二维码登录实现](https://github.com/liuchuancong/pure_live/blob/master/lib/modules/account/bilibili/qr_login_controller.dart)
- [Bilibili API Collect：直播间信息与画质 QN](https://github.com/pskdje/bilibili-API-collect/blob/main/docs/live/info.md)
- [yt-dlp 当前 Bilibili 直播画质映射与取流实现](https://github.com/yt-dlp/yt-dlp/blob/master/yt_dlp/extractor/bilibili.py)
- [keyring Rust API](https://docs.rs/keyring/4.2.0)
- [Tauri Clipboard 插件与权限](https://v2.tauri.app/plugin/clipboard/)
- [mpegts.js 配置 API](https://xqq.im/mpegts.js/docs/api.html)
