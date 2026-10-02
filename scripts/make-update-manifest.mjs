#!/usr/bin/env node
// 生成 Tauri 更新器的 latest.json 清单。
//
// 用法：node scripts/make-update-manifest.mjs <version> <release-assets-dir>
//
// 读取 release-assets 中的更新器签名文件（构建时由 TAURI_SIGNING_PRIVATE_KEY
// 生成），拼出指向 GitHub Release 资产的下载地址。任一必需平台的签名缺失时
// 打印警告；全部缺失则跳过生成（保持无签名密钥的 CI 流程可用）。

import { readdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";

const [, , versionArgument, assetsDir] = process.argv;
if (!versionArgument || !assetsDir) {
  console.error("用法：node scripts/make-update-manifest.mjs <version> <release-assets-dir>");
  process.exit(1);
}

const version = versionArgument.replace(/^v/u, "");
const repo = "Kihara-Ri/Qiliu";
const files = readdirSync(assetsDir);

function findSignature(pattern) {
  return files.find(name => pattern.test(name));
}

const platformEntries = [
  {
    // tauri-plugin-updater 的平台键格式为 {os}-{arch} 全称：
    // darwin-aarch64 / windows-x86_64，见插件 updater_arch()/updater_os()。
    platform: "darwin-aarch64",
    // 架构大小写在 runner 上可能不同（ARM64/arm64），按前缀匹配 .app.tar.gz.sig。
    signature: findSignature(new RegExp(`^Qiliu-${version}-macOS-\\S+\\.app\\.tar\\.gz\\.sig$`, "u")),
    assetPattern: new RegExp(`^Qiliu-${version}-macOS-\\S+\\.app\\.tar\\.gz$`, "u"),
  },
  {
    platform: "windows-x86_64",
    signature: findSignature(new RegExp(`^Qiliu-${version}-Windows-x64-setup\\.exe\\.sig$`, "iu")),
    assetPattern: new RegExp(`^Qiliu-${version}-Windows-x64-setup\\.exe$`, "iu"),
  },
];

const platforms = {};
for (const entry of platformEntries) {
  if (!entry.signature) {
    console.warn(`缺少 ${entry.platform} 的更新器签名，latest.json 将不包含该平台。`);
    console.warn("这通常表示构建时没有配置 TAURI_SIGNING_PRIVATE_KEY secret。");
    continue;
  }
  const signature = readFileSync(join(assetsDir, entry.signature), "utf8").trim();
  if (!signature) {
    console.warn(`${entry.platform} 的签名文件为空，跳过。`);
    continue;
  }
  const asset = files.find(name => entry.assetPattern.test(name) && !name.endsWith(".sig"));
  if (!asset) {
    console.warn(`找到 ${entry.platform} 的签名但没有对应的安装包资产，跳过。`);
    continue;
  }
  platforms[entry.platform] = {
    signature,
    url: encodeURI(`https://github.com/${repo}/releases/download/v${version}/${asset}`),
  };
}

if (Object.keys(platforms).length === 0) {
  console.warn("没有任何平台带有更新器签名：跳过 latest.json 生成（应用内更新保持不可用，其余发布流程不受影响）。");
  process.exit(0);
}

const manifest = {
  version,
  pub_date: new Date().toISOString(),
  platforms,
};

const outputPath = join(assetsDir, "latest.json");
writeFileSync(outputPath, `${JSON.stringify(manifest, null, 2)}\n`);
console.log(`已生成 ${outputPath}（平台：${Object.keys(platforms).join(", ")}）`);
