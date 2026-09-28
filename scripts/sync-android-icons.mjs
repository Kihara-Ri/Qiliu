import { access, cp } from "node:fs/promises";
import { constants } from "node:fs";

// `tauri android init` (re)generates gen/android with the Tauri template's
// default launcher icons. The project's own icons live in
// src-tauri/icons/android and must be copied over the generated res/ every
// time the Android project is (re)initialized — locally and in CI — or the
// published APK ships with the default Tauri icon.
const source = new URL("../src-tauri/icons/android/", import.meta.url);
const target = new URL("../src-tauri/gen/android/app/src/main/res/", import.meta.url);

await access(new URL("mipmap-xxxhdpi/ic_launcher.png", source), constants.R_OK).catch(() => {
  throw new Error(
    "缺少 src-tauri/icons/android 图标。请先提交 Android 图标资源（可用 `npm run tauri -- icon app-icon.svg` 生成）。",
  );
});
await access(target, constants.W_OK).catch(() => {
  throw new Error("未找到 src-tauri/gen/android/app/src/main/res/。请先运行 `npm run android:init`。");
});

for (const density of [
  "mipmap-mdpi",
  "mipmap-hdpi",
  "mipmap-xhdpi",
  "mipmap-xxhdpi",
  "mipmap-xxxhdpi",
  "mipmap-anydpi-v26",
]) {
  await cp(new URL(`${density}/`, source), new URL(`${density}/`, target), { recursive: true });
}
await cp(
  new URL("values/ic_launcher_background.xml", source),
  new URL("values/ic_launcher_background.xml", target),
);

console.log("已将 src-tauri/icons/android 同步到 gen/android 启动器图标。");
