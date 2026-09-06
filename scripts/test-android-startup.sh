#!/usr/bin/env bash
set -euo pipefail
project_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source "$project_dir/scripts/android-env.sh"
apk="${1:?用法：bash scripts/test-android-startup.sh APK路径 [模拟器序列号]}"
serial="${2:-emulator-5554}"
if [[ "$serial" != emulator-* ]]; then
  echo "启动回归测试只运行于模拟器，请指定 emulator-* 序列号。" >&2
  exit 1
fi
app_id="com.kiharari.simplelive"
report_dir="$project_dir/release/android-startup-test"
mkdir -p "$report_dir"
adb -s "$serial" wait-for-device
adb -s "$serial" install -r "$apk"
adb -s "$serial" logcat -c
adb -s "$serial" logcat -b crash -c
for attempt in 1 2 3; do
  adb -s "$serial" shell am force-stop "$app_id"
  adb -s "$serial" shell am start -W -n "$app_id/.MainActivity" > "$report_dir/start-$attempt.txt"
  sleep 8
  adb -s "$serial" shell pidof "$app_id" > "$report_dir/pid-$attempt.txt" || true
  if [[ ! -s "$report_dir/pid-$attempt.txt" ]]; then
    adb -s "$serial" logcat -d > "$report_dir/logcat.txt"
    echo "第 $attempt 次启动后进程已退出，日志：$report_dir/logcat.txt" >&2
    exit 1
  fi
  echo "第 $attempt 次冷启动通过"
done
adb -s "$serial" shell input keyevent KEYCODE_HOME
sleep 2
adb -s "$serial" shell am start -W -n "$app_id/.MainActivity" > "$report_dir/resume.txt"
sleep 5
adb -s "$serial" shell pidof "$app_id" > "$report_dir/resume-pid.txt" || true
adb -s "$serial" logcat -b crash -d > "$report_dir/crash.txt"
adb -s "$serial" logcat -d > "$report_dir/logcat.txt"
adb -s "$serial" exec-out screencap -p > "$report_dir/screen.png"
if [[ ! -s "$report_dir/resume-pid.txt" ]]; then
  echo "后台恢复后进程已退出。" >&2
  exit 1
fi
if grep -E 'FATAL EXCEPTION|Fatal signal|Abort message' "$report_dir/crash.txt"; then
  echo "检测到崩溃日志，测试失败。" >&2
  exit 1
fi
echo "三次冷启动与后台恢复通过，日志和截图：$report_dir"
