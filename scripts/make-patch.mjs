#!/usr/bin/env node
// 补丁（体验补丁）制作与签名工具。
//
// 用法：
//   node scripts/make-patch.mjs --generate-key
//     生成 ed25519 密钥对；私钥写入 ~/.qiliu/patch-private.hex（不会进入仓库），
//     公钥打印出来，需要填入 src-tauri/src/patch_hub.rs 的 PATCH_PUBKEY_HEX。
//
//   node scripts/make-patch.mjs --make --sequence 3 --notes "修复……" --source patches/src/3
//     读取 source 目录（扁平，只允许安全文件名）中的文件，复制到 patches/3/，
//     并重新生成签名后的 patches/index.json。私钥从环境变量
//     QILIU_PATCH_PRIVATE_KEY（64 位 hex 种子）或 ~/.qiliu/patch-private.hex 读取。
//
//   node scripts/make-patch.mjs --baseline
//     生成 sequence 为 0 的空基线 index.json（表示“当前没有补丁”）。
//
//   node scripts/make-patch.mjs --fixture
//     用固定测试种子生成 src-tauri/tests/patch-index-fixture.json，
//     供 Rust 单元测试验证“Node 签名 / Rust 验签”的跨实现一致性。
//
// 签名规范：对 payload 的规范化 JSON（键按字节序递归排序、紧凑序列化）做
// ed25519 签名。这与 Rust 端 serde_json::Value 的 BTreeMap 序列化结果一致。

import {
  createHash,
  createPrivateKey,
  createPublicKey,
  generateKeyPairSync,
  sign as cryptoSign,
} from "node:crypto";
import { existsSync, mkdirSync, readFileSync, readdirSync, renameSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const projectRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const privateKeyPath = join(homedir(), ".qiliu", "patch-private.hex");
const patchRoot = join(projectRoot, "patches");
const fixturePath = join(projectRoot, "src-tauri", "tests", "patch-index-fixture.json");
const fixtureSeedHex = "01".repeat(32);

const args = process.argv.slice(2);
const flag = (name) => args.includes(name);
const option = (name) => {
  const index = args.indexOf(name);
  return index >= 0 ? args[index + 1] : undefined;
};

function canonicalize(value) {
  if (value === null || typeof value !== "object") return value;
  if (Array.isArray(value)) return value.map(canonicalize);
  const out = {};
  for (const key of Object.keys(value).sort()) out[key] = canonicalize(value[key]);
  return out;
}

function canonicalJson(payload) {
  // 去掉 U+2028/U+2029：JSON.stringify 与 serde_json 对它们的处理不一致。
  return JSON.stringify(canonicalize(payload)).replace(/\u2028|\u2029/gu, "");
}

function privateKeyFromSeedHex(seedHex) {
  if (!/^[0-9a-f]{64}$/u.test(seedHex)) {
    throw new Error("私钥格式不正确：需要 64 位 hex（32 字节 ed25519 种子）");
  }
  const der = Buffer.from(`302e020100300506032b657004220420${seedHex}`, "hex");
  return createPrivateKey({ key: der, format: "der", type: "pkcs8" });
}

function publicKeyHexOf(privateKey) {
  const spki = createPublicKey(privateKey).export({ type: "spki", format: "der" });
  return spki.subarray(-32).toString("hex");
}

function signPayloadHex(payload, privateKey) {
  const signature = cryptoSign(null, Buffer.from(canonicalJson(payload), "utf8"), privateKey);
  return signature.toString("hex");
}

function sha256Hex(data) {
  return createHash("sha256").update(data).digest("hex");
}

function isSafeFileName(name) {
  return /^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$/u.test(name) && !name.includes("..");
}

function loadPrivateKey() {
  const fromEnv = process.env.QILIU_PATCH_PRIVATE_KEY;
  if (fromEnv) return privateKeyFromSeedHex(fromEnv.trim());
  if (existsSync(privateKeyPath)) {
    return privateKeyFromSeedHex(readFileSync(privateKeyPath, "utf8").trim());
  }
  throw new Error(`未找到补丁私钥：请设置 QILIU_PATCH_PRIVATE_KEY，或先运行 --generate-key（密钥文件：${privateKeyPath}）`);
}

function writeJsonAtomic(targetPath, json) {
  mkdirSync(dirname(targetPath), { recursive: true });
  const temporary = `${targetPath}.tmp`;
  writeFileSync(temporary, `${JSON.stringify(json, null, 2)}\n`);
  renameSync(temporary, targetPath);
}

function currentAppVersion() {
  const packageJson = JSON.parse(readFileSync(join(projectRoot, "package.json"), "utf8"));
  return packageJson.version;
}

function buildPayload({ sequence, notes, minAppVersion, maxAppVersion, files }) {
  const payload = {
    schema: 1,
    sequence,
    minAppVersion,
    maxAppVersion: maxAppVersion ?? null,
    notes,
    baseUrl: `https://raw.githubusercontent.com/Kihara-Ri/Qiliu/main/patches/${sequence}/`,
    files,
  };
  return JSON.parse(canonicalJson(payload));
}

function generateKey() {
  const { privateKey } = generateKeyPairSync("ed25519");
  const seedHex = privateKey.export({ type: "pkcs8", format: "der" }).subarray(-32).toString("hex");
  const publicKeyHex = publicKeyHexOf(privateKey);
  mkdirSync(dirname(privateKeyPath), { recursive: true });
  writeFileSync(privateKeyPath, `${seedHex}\n`, { mode: 0o600 });
  console.log(`补丁私钥已写入 ${privateKeyPath}（请自行备份，不要提交到仓库）`);
  console.log("补丁公钥（填入 src-tauri/src/patch_hub.rs 的 PATCH_PUBKEY_HEX）：");
  console.log(publicKeyHex);
}

function makePatch() {
  const sequence = Number(option("--sequence"));
  const notes = (option("--notes") ?? "").replace(/\u2028|\u2029/gu, "");
  const source = option("--source");
  const minAppVersion = option("--min-app-version") ?? currentAppVersion();
  const maxAppVersion = option("--max-app-version") ?? null;
  if (!Number.isInteger(sequence) || sequence < 1) {
    throw new Error("--sequence 需要是不小于 1 的整数");
  }
  if (!source || !existsSync(source)) {
    throw new Error(`--source 目录不存在：${source}`);
  }
  const files = {};
  for (const name of readdirSync(source)) {
    if (!isSafeFileName(name)) {
      throw new Error(`补丁目录里有不安全的文件名（只允许字母数字、点、横线、下划线）：${name}`);
    }
    files[name] = sha256Hex(readFileSync(join(source, name)));
  }
  if (!files["entry.js"]) {
    throw new Error("补丁必须包含 entry.js 作为入口模块");
  }
  const payload = buildPayload({ sequence, notes, minAppVersion, maxAppVersion, files });
  const privateKey = loadPrivateKey();
  const envelope = { schema: 1, payload, signature: signPayloadHex(payload, privateKey) };

  const targetDir = join(patchRoot, String(sequence));
  mkdirSync(targetDir, { recursive: true });
  for (const name of Object.keys(files)) {
    writeFileSync(join(targetDir, name), readFileSync(join(source, name)));
  }
  writeJsonAtomic(join(patchRoot, "index.json"), envelope);
  console.log(`已生成补丁 v${sequence}：patches/${sequence}/ 与 patches/index.json`);
}

function makeBaseline() {
  const privateKey = loadPrivateKey();
  const payload = buildPayload({
    sequence: 0,
    notes: "",
    minAppVersion: "1.0.0",
    maxAppVersion: null,
    files: {},
  });
  writeJsonAtomic(join(patchRoot, "index.json"), {
    schema: 1,
    payload,
    signature: signPayloadHex(payload, privateKey),
  });
  console.log("已生成空基线 patches/index.json（sequence 0）");
}

function makeFixture() {
  const privateKey = privateKeyFromSeedHex(fixtureSeedHex);
  const entrySource = [
    "// Rust 单元测试使用的补丁入口 fixture：仅验证加载管线，不做任何实际修改。",
    "export default function apply() {",
    "  return Promise.resolve(\"fixture-applied\");",
    "}",
    "",
  ].join("\n");
  const entryBytes = Buffer.from(entrySource, "utf8");
  const payload = buildPayload({
    sequence: 7,
    notes: "fixture：仅供 Rust 单元测试使用",
    minAppVersion: "1.0.0",
    maxAppVersion: null,
    files: { "entry.js": sha256Hex(entryBytes) },
  });
  const fixture = {
    description: "由 scripts/make-patch.mjs --fixture 生成；对应测试种子 0x01*32，请勿用于生产。",
    testPublicKeyHex: publicKeyHexOf(privateKey),
    // canonicalJson 是脚本端对 payload 的规范化序列化结果；
    // Rust 测试用它断言两端规范化逐字节一致（签名校验之外的第二道跨实现对拍）。
    canonicalJson: canonicalJson(payload),
    envelope: { schema: 1, payload, signature: signPayloadHex(payload, privateKey) },
    files: { "entry.js": entryBytes.toString("base64") },
  };
  mkdirSync(dirname(fixturePath), { recursive: true });
  writeJsonAtomic(fixturePath, fixture);
  console.log(`已生成测试 fixture：${fixturePath}`);
  console.log(`测试公钥：${fixture.testPublicKeyHex}`);
}

function main() {
  if (flag("--generate-key")) return generateKey();
  if (flag("--make")) return makePatch();
  if (flag("--baseline")) return makeBaseline();
  if (flag("--fixture")) return makeFixture();
  console.log("用法：node scripts/make-patch.mjs --generate-key | --make --sequence N --notes S --source DIR | --baseline | --fixture");
}

try {
  main();
} catch (error) {
  console.error(error instanceof Error ? error.message : error);
  process.exit(1);
}
