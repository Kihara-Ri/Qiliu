# Android 本地构建与启动测试

macOS 的 SDK 默认位于 `~/Library/Android/sdk`。本项目使用 Java 21、NDK 28.2.13676358 和 ARM64 Android 模拟器。环境脚本只影响当前终端，不修改系统配置。

```sh
source scripts/android-env.sh
npm run package:android
```

首次构建会自动生成 Tauri Android 工程。调试 APK 输出在 `release/`，可安装到 ARM64 Android 设备或模拟器。

## 启动回归

启动名为 `Qiliu_API_35` 的模拟器后执行：

```sh
source scripts/android-env.sh
emulator -avd Qiliu_API_35 -no-snapshot &
bash scripts/test-android-startup.sh release/Qiliu-1.2.0-Android-arm64-debug.apk emulator-5554
```

脚本只接受模拟器序列号，检查三次强制停止后的启动以及一次后台恢复。日志、进程检查结果和截图保存在 `release/android-startup-test/`。进程存活检查不等同于直播播放验证；需结合截图确认界面正常显示。

## 启动崩溃修复

锁定的 Tao 0.35 不提供 `ndk-context` 初始化，而 Android Keyring 调用它时会触发 `android context was not initialized`。本项目在 Tauri setup 阶段取得 Application 上下文，保留 JNI 全局引用并初始化凭据存储。升级到自动初始化上下文的 Tao 版本时，应一并移除这段适配，避免重复初始化。
