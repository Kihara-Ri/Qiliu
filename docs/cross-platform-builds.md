# 跨平台界面与构建

## 移动界面

栖流的移动端沿用“画面优先”的影院式深色语言，但不直接缩放桌面侧栏：

- 竖屏中，视频保持全画幅；设置以底部抽屉出现，高度不超过视口的 78%。
- 设置、收藏、监控、关于固定在抽屉底部，图标与文字组成至少 44px 的触控目标。
- 音量与设置位于左右下角。触屏点击音量时同时展开滑杆，仍保留静音切换。
- 打开面板时不自动聚焦链接输入框，避免软键盘遮住上下文；用户点击输入框后可直接粘贴并清洗分享文案。
- 横屏且高度较小时，设置层改为全屏，避免短视口里的双重滚动。
- 桌面仍使用永久挂载的右侧面板；两套布局都只改变 `transform`，不调整视频层尺寸。

## 本地快速构建

先安装依赖并验证：

```bash
npm ci
npm run check
```

当前桌面系统一键构建：

```bash
npm run package:current
```

- macOS：构建 `.app`，再生成 ad-hoc 签名的 DMG。
- Windows：在 Windows 主机生成当前用户安装的 NSIS `.exe`。

Android 调试 APK：

```bash
export ANDROID_HOME="$HOME/Library/Android/sdk" # macOS 示例
npm run package:android
```

Android release APK 还需要四个环境变量：

```text
ANDROID_KEYSTORE_PATH
ANDROID_KEY_ALIAS
ANDROID_KEY_PASSWORD
ANDROID_STORE_PASSWORD
```

设置后运行：

```bash
npm run package:android:release
```

## 每次发版

```bash
npm run release:prepare -- 1.2.0
```

脚本会同步 `package.json`、lockfile、Tauri、Cargo 和关于页版本，并运行完整检查。确认更新记录后提交并推送标签：

```bash
git add .
git commit -m "Release Qiliu 1.2.0"
git tag v1.2.0
git push origin main v1.2.0
```

`Release` 工作流会在原生 GitHub 托管环境分别生成：

- macOS DMG；
- Windows x64 NSIS 安装程序；
- Android arm64 APK；

随后把三个安装包上传到同一个 GitHub Release。未配置 Android 签名 Secret 时会明确生成 `-debug.apk`，适合侧载测试，但后续正式版本应使用固定的 release keystore，才能原地覆盖升级。

## GitHub Android 签名 Secret

在仓库的 Actions Secrets 中配置：

```text
ANDROID_KEYSTORE_BASE64
ANDROID_KEY_ALIAS
ANDROID_KEY_PASSWORD
ANDROID_STORE_PASSWORD
```

其中 `ANDROID_KEYSTORE_BASE64` 是 keystore 文件的 Base64 内容。keystore 不得提交到公开仓库；密码也不得写入 workflow 或日志。

## 验证边界

- `npm run check` 验证业务逻辑、前端构建和当前主机的 Rust 代码。
- GitHub Actions 在 Windows/Android 原生工具链完成真实编译，不能用 macOS 的交叉编译结果代替。
- 安装包生成成功不等于真实直播长时间播放通过；发布前仍需在对应系统/设备测试一个虎牙直播间和一个 Bilibili 直播间。
