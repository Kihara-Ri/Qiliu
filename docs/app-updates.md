# 栖流的应用内更新体系

栖流有两条互相独立的更新通道：**体验补丁**（自动，无需确认）与**应用版本更新**（需在客户端确认）。两条通道都不触碰用户数据。

## 总览

| | 体验补丁 | 应用版本更新 |
| --- | --- | --- |
| 定位 | 改善体验的小改动，只覆盖前端层 | 完整应用的新版本（可含 Rust 端改动） |
| 分发位置 | 仓库 `patches/` 目录（main 分支） | GitHub Releases 安装包 |
| 触发 | 检测到即自动下载并应用，无需确认 | 客户端内展示新版本，用户确认后更新 |
| 检查时机 | 启动后延迟一次、之后每 6 小时、手动 | 同左（同一次检查一并完成） |
| 安装方式 | 写入应用数据目录，经自定义协议动态加载 | 桌面端：下载→minisign 验签→安装→自动重启；Android：打开发布页 |
| 用户控制 | 设置中可停用；「关于」中可回滚 | 「关于」面板确认或忽略 |
| 安全机制 | ed25519 索引验签 + 逐文件 sha256 + 域名白名单 + 加载失败自动停用 | minisign 安装包验签（tauri-plugin-updater） |

### 数据为什么不会丢

- 收藏、上次播放、音量、控件位置保存在 **localStorage**；Webview 数据目录按 bundle identifier（`com.kiharari.simplelive`）键控，与安装包位置无关。
- Bilibili 登录凭据保存在**系统安全存储**（macOS 钥匙串 / Windows 凭据管理器 / Android Keystore），不在应用包内。
- 体验补丁只写入应用数据目录的 `patches/` 子目录，从不修改安装包本体（macOS bundle 签名保持完整）。
- 版本更新只替换应用本体；应用数据目录与安全存储不动。

## 体验补丁通道

### 工作原理

1. 应用从 `patches/index.json`（raw.githubusercontent.com，jsdelivr 备用）读取补丁索引。
2. 索引 `{ schema, payload, signature }` 的 `payload` 必须通过 **ed25519 验签**，公钥内嵌于 `src-tauri/src/patch_hub.rs` 的 `PATCH_PUBKEY_HEX`。
3. 通过 `minAppVersion` / `maxAppVersion`（可选）判断是否适用于当前应用版本。
4. 逐文件下载（2MB/文件、8MB 总量上限），**sha256 逐文件校验**，暂存目录原子激活。
5. 前端通过 `qiliu-patch://localhost/entry.js`（Windows/Android 为 `http://qiliu-patch.localhost/...`）动态 `import()` 入口模块；协议只提供激活清单内哈希一致的文件。
6. 入口模块约定：默认导出 `apply(api)`，`api = { version, notes, log }`。模块也可以只有副作用。
7. 加载失败会上报；**连续 2 次失败自动停用补丁**，应用继续以未打补丁状态运行。用户可在「设置 → 更新」重新开启，或在「关于」中回滚（回滚后同版本不会自动回来，只有更新的补丁序列才会）。

### 制作与发布补丁

```bash
# 首次：生成密钥（私钥在 ~/.qiliu/patch-private.hex，不进仓库）
node scripts/make-patch.mjs --generate-key
# 把打印出的公钥填入 src-tauri/src/patch_hub.rs 的 PATCH_PUBKEY_HEX（换钥需发版）

# 编写补丁：patches/src/<N>/entry.js（扁平目录，必须含 entry.js）
node scripts/make-patch.mjs --make --sequence <N> --notes "修复……" --source patches/src/<N>
# 可选门槛：--min-app-version 1.5.0 --max-app-version 1.6.0

git add patches/<N> patches/index.json && git commit && git push
```

推送 main 后，客户端在下次检查时自动获取。签名规范：对 payload 的规范化 JSON（键递归按字节序排序、紧凑序列化）做 ed25519 签名——Rust 端 `serde_json::Value` 的 BTreeMap 序列化与此逐字节一致，`src-tauri` 测试里有跨实现对拍。

## 应用版本更新通道

### 桌面端（macOS / Windows）

- 使用 `tauri-plugin-updater`，端点 `https://github.com/Kihara-Ri/Qiliu/releases/latest/download/latest.json`。
- CI 在配置了 `TAURI_SIGNING_PRIVATE_KEY` secret 时，以 `--config src-tauri/tauri.updater.conf.json` 叠加开启 `createUpdaterArtifacts` 构建，为 macOS 产出 `Qiliu-<版本>-macOS-<架构>.app.tar.gz`、为 Windows 产出 NSIS 安装包的 `.sig` 签名，并由 `scripts/make-update-manifest.mjs` 汇总成 `latest.json` 一并上传。
- 更新器工件**只在 CI 配置密钥时开启**：本地打包与未配置密钥的 CI 走普通构建（`npm run tauri build` 不受影响），发布流程照常，只是没有 `.sig` 与 `latest.json`。
- 客户端流程：静默检查（自动/手动）→ 「关于」面板出现新版本卡片 → 用户点「下载并重启」→ 下载（带进度）→ minisign 验签 → 安装 → 自动重启。Windows 上安装器以静默模式运行。
- secret 未配置时 CI 正常发布安装包，只是不生成 `latest.json`，应用内更新保持「已是最新」。

### Android

系统不允许应用自行替换 APK：客户端检查 GitHub 最新 Release，出现新版本时提供「前往发布页」按钮，下载同签名 APK 覆盖安装即可，数据保留。（注意 README 中的 debug/release 签名差异说明。）

### 一次性密钥设置（维护者）

1. `npx tauri signer generate -w ~/.qiliu/qiliu-updater.key`（本机已生成；密码可留空）。
2. 仓库 Settings → Secrets → Actions 添加：
   - `TAURI_SIGNING_PRIVATE_KEY`：`~/.qiliu/qiliu-updater.key` 文件的完整内容；
   - `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`：密钥密码（留空则可不加）。
3. 公钥已写入 `src-tauri/tauri.conf.json` 的 `plugins.updater.pubkey`。

**丢失私钥 = 无法再发应用内更新**（只能换钥并发新版本）。请把两个私钥（`~/.qiliu/qiliu-updater.key` 与 `~/.qiliu/patch-private.hex`）备份到安全位置。

## 相关文件

- `src-tauri/src/patch_hub.rs` — 补丁拉取/验签/存储/协议/命令与单元测试
- `src-tauri/src/app_update.rs` — 版本检查/确认安装/打开发布页
- `src/app-updates.ts` — 前端编排（加载补丁、检查节奏、安装进度 UI）
- `src/update-logic.ts` — 纯展示逻辑与对应测试
- `scripts/make-patch.mjs` — 补丁签名/发布工具
- `scripts/make-update-manifest.mjs` — latest.json 生成（CI publish 阶段）
- `.github/workflows/release.yml` — 更新器工件与清单产出（密钥存在时叠加 `src-tauri/tauri.updater.conf.json`）
- `patches/index.json` — 补丁索引（sequence 0 为空基线）
