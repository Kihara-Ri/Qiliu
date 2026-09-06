#!/usr/bin/env bash

set -euo pipefail

project_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
version="$(node -p "require('$project_dir/package.json').version")"
mode="${1:-debug}"

source "$project_dir/scripts/android-env.sh"

if [[ ! -d "$ANDROID_HOME" || ! -d "$NDK_HOME" ]]; then
  echo "未找到 Android SDK / NDK。请先安装工具或设置 ANDROID_HOME 与 NDK_HOME。" >&2
  exit 1
fi

cd "$project_dir"
if [[ ! -d src-tauri/gen/android ]]; then
  npm run android:init
fi

if [[ "$mode" == "--release" ]]; then
  node scripts/configure-android-signing.mjs
  npm run tauri -- android build --apk --target aarch64 --ci
  suffix=""
else
  npm run tauri -- android build --debug --apk --target aarch64 --ci
  suffix="-debug"
fi

apk_path="$(find src-tauri/gen/android/app/build/outputs/apk -type f -name '*.apk' | head -n 1)"
if [[ -z "$apk_path" ]]; then
  echo "Android 构建完成，但没有找到 APK。" >&2
  exit 1
fi

mkdir -p release
output="release/Qiliu-$version-Android-arm64$suffix.apk"
cp "$apk_path" "$output"
shasum -a 256 "$output"
