# 栖流：从修复到 GitHub Release 的完整流程

本次版本为 **1.4.1**。正式产物对应 Git 标签 `v1.4.1`，GitHub Actions 的 Release 工作流使用这个标签的代码构建。故障分析见 [调查报告](live-playback-fix-2026-09-28.md)。

## 1. 固定发布内容

本次包含虎牙 FLV 预加载交接、系统播放信息、直播间封面，以及工作区此前完成的 CCTV-1／CCTV-13 支持、Android 启动器图标与 release 编译修正。发布核对时补齐央视 CDN 的 CSP 许可，并让本地 Android 签名脚本识别 `ANDROID_KEYSTORE_PATH`。

先核对差异和远端版本，确保没有凭据、临时签名 URL 或生成目录被提交：

```bash
git status --short
git diff --stat
git diff --check
git ls-remote origin HEAD refs/heads/main 'refs/tags/v*'
```

不要通过删除或覆盖旧标签更新已有版本。若本地有其他任务的修改，应先明确发布范围；提交列表应与本次版本说明一致。

## 2. 同步版本和验证

```bash
npm ci
npm run release:prepare -- 1.4.1
```

版本脚本同步 `package.json`、`package-lock.json`、Tauri 配置、Cargo 配置与关于页。随后检查版本一致性、前端测试、生产构建、Rust 测试和差异格式。Cargo 执行后更新的 `Cargo.lock` 也应纳入版本提交。

网络测试默认不随普通检查运行；本次虎牙网络测量和原生播放器观察详见调查报告。发布前应保留“已测环境”和“未测环境”的区分。

## 3. 提交并推送标签

将经过审阅的源代码、测试、报告、版本文件和发布流程纳入暂存；不提交 `dist/`、`node_modules/`、`src-tauri/target/`、Android 生成工程或签名密钥。

```bash
git diff --cached --stat
git diff --cached --check
git commit -m "Release Qiliu 1.4.1: verify complete macOS bundle signing"
git tag v1.4.1
git push --atomic origin main v1.4.1
```

`--atomic` 要求分支与标签一起推送成功。本仓库的发布触发条件是推送 `v*` 标签，单独推送普通提交不会生成安装包。若 HTTPS 凭据不可用而本机已配置 GitHub SSH，可把命令中的 `origin` 换成 `git@github.com:Kihara-Ri/Qiliu.git`，不需要把凭据写入项目。

## 4. GitHub Actions 编译各平台

流程定义：`.github/workflows/release.yml`。

| 任务 | 构建环境 | 校验与编译 | 安装包 |
| --- | --- | --- | --- |
| macOS | macos-14 / ARM64 | `npm ci` → `npm run check` → Tauri release `.app` → 完整 app 的 ad-hoc 签名与验证 → DMG → `hdiutil verify` | `Qiliu-1.4.1-macOS-ARM64.dmg` |
| Windows | windows-latest / x64 | `npm ci` → `npm run check` → Tauri release NSIS | `Qiliu-1.4.1-Windows-x64-setup.exe` |
| Android | ubuntu-latest + Java 17 + Android / Rust ARM64 工具链 | 前端检查 → 初始化 Android → 同步图标 → 配置签名 → release APK | `Qiliu-1.4.1-Android-arm64.apk` 或 `Qiliu-1.4.1-Android-arm64-debug.apk` |

macOS 当前只构建 Apple Silicon 版本，不是 Intel 或 Universal 包。没有额外提供 Linux 和 iOS 发布目标。

Android 始终使用 release 编译；文件名 `-debug` 表示使用调试密钥签名，不表示使用 debug 编译。正式签名通过仓库 Secret 注入；密码和 keystore 不得进入代码或日志。临时 CI 调试密钥可能与旧安装包不同，不能承诺覆盖升级；应先保留收藏等数据。macOS 包未经过 Apple 公证。

本地 macOS 安装包可由以下命令生成：

```bash
npm run package:current
hdiutil verify 'release/栖流 1.4.1.dmg'
```

不要把旧 `.app` 直接重命名当作新版本安装包；必须先编译新标签对应代码，再执行打包。

## 5. 上传到 GitHub Release

三个构建任务分别上传安装包 artifact。仅在所有平台成功后，`Publish GitHub Release` 任务才开始：

1. 下载三个平台的安装包。
2. 加入本次调查报告和完整发布流程 Markdown 文件。
3. 从 `CHANGELOG.md` 提取对应版本说明。
4. 创建对应标签的 GitHub Release，上传安装包和报告。

发布页：<https://github.com/Kihara-Ri/Qiliu/releases/tag/v1.4.1>。该地址是发布目标；只有工作流完成且实际资产可下载后才算交付完成。

## 6. 核验发布结果

不能只看标签、提交或“构建已启动”。应确认：

- Release 工作流及 macOS、Windows、Android、Publish 任务全部成功。
- Release 不是草稿，`tag_name` 为 `v1.4.1`，标签指向本次发布提交。
- 资产列表包含三种预期安装包和两份报告，名称、版本、平台、签名后缀正确，文件大小非零。
- 对每个实际 `browser_download_url` 发起跟随跳转的下载检查，最终 HTTP 200。
- 下载 macOS DMG 并执行 `hdiutil verify`；检查其中应用版本。Windows/Android 的打包成功和下载成功不等于真机播放通过。

可使用 `gh run view`、`gh release view`，或公开 GitHub API 查询状态。若 CLI 登录失效，仍可用已配置的 SSH 推送，并用公开 API核验；不要复制密码或令牌到聊天中。

## 7. 本次发布核验发现的打包问题

第一次发布 `v1.4.0` 后，下载 GitHub macOS DMG 并只读挂载检查，包内版本为 1.4.0，但 `codesign --verify --deep --strict` 返回 `code has no resources but signature indicates they must be present`。镜像校验成功不能代替应用资源签名校验。

本地打包脚本已对整个 app 进行 ad-hoc 签名且验证通过，因此修正 CI 采用相同步骤：先生成 `.app`，运行 `package:macos` 为完整 bundle 签名并验证，再创建 DMG 和检查镜像。以新的 `v1.4.1` 标签重新编译三个平台，不改写已公开的 1.4.0 标签。Windows 和 Android 也重新编译以保持版本一致。此签名修复不等于 Apple 公证。

## 8. 失败处理

单个平台失败时先读取该任务日志。若是网络或 runner 临时故障，可重跑原任务；若必须修改代码，新增修复提交并发布新的版本标签，避免把已公开版本静默替换为不同源码。发布完成后，交付说明应给出提交、标签、工作流、下载链接、签名状态和仍未完成的设备验收。
