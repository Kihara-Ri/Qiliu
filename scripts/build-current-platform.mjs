import { spawnSync } from "node:child_process";

const npmCommand = process.platform === "win32" ? "npm.cmd" : "npm";
const script = process.platform === "darwin"
  ? "package:macos"
  : process.platform === "win32"
    ? "package:windows"
    : null;

if (!script) {
  throw new Error("当前脚本只负责 macOS/Windows；Android 请运行 npm run package:android");
}

const check = spawnSync(npmCommand, ["run", "check"], { stdio: "inherit" });
if (check.status !== 0) process.exit(check.status ?? 1);

if (process.platform === "darwin") {
  const build = spawnSync(npmCommand, ["run", "tauri", "--", "build", "--bundles", "app"], { stdio: "inherit" });
  if (build.status !== 0) process.exit(build.status ?? 1);
}

const packaged = spawnSync(npmCommand, ["run", script], { stdio: "inherit" });
process.exit(packaged.status ?? 1);
