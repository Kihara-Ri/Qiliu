import { readFile, writeFile } from "node:fs/promises";

const CHECK_FLAG = "--check";
const requested = process.argv[2];
const packagePath = new URL("../package.json", import.meta.url);
const lockPath = new URL("../package-lock.json", import.meta.url);
const tauriPath = new URL("../src-tauri/tauri.conf.json", import.meta.url);
const cargoPath = new URL("../src-tauri/Cargo.toml", import.meta.url);
const indexPath = new URL("../index.html", import.meta.url);

const packageJson = JSON.parse(await readFile(packagePath, "utf8"));
const version = requested && requested !== CHECK_FLAG ? requested : packageJson.version;

if (!/^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/.test(version)) {
  throw new Error(`版本号不符合 SemVer：${version}`);
}

const lock = JSON.parse(await readFile(lockPath, "utf8"));
const tauri = JSON.parse(await readFile(tauriPath, "utf8"));
const cargo = await readFile(cargoPath, "utf8");
const index = await readFile(indexPath, "utf8");

const current = {
  package: packageJson.version,
  lock: lock.version,
  lockRoot: lock.packages?.[""]?.version,
  tauri: tauri.version,
  cargo: cargo.match(/^version = "([^"]+)"/m)?.[1],
  about: index.match(/Qiliu · ([^ ·<]+) · AGPL/)?.[1],
};

if (requested === CHECK_FLAG) {
  const mismatches = Object.entries(current).filter(([, value]) => value !== version);
  if (mismatches.length) {
    throw new Error(`版本号未同步：${mismatches.map(([name, value]) => `${name}=${value}`).join(", ")}`);
  }
  console.log(`版本号已同步：${version}`);
  process.exit(0);
}

packageJson.version = version;
lock.version = version;
if (lock.packages?.[""]) lock.packages[""].version = version;
tauri.version = version;

const nextCargo = cargo.replace(/^version = "[^"]+"/m, `version = "${version}"`);
const nextIndex = index.replace(/Qiliu · [^ ·<]+ · AGPL/, `Qiliu · ${version} · AGPL`);

await Promise.all([
  writeFile(packagePath, `${JSON.stringify(packageJson, null, 2)}\n`),
  writeFile(lockPath, `${JSON.stringify(lock, null, 2)}\n`),
  writeFile(tauriPath, `${JSON.stringify(tauri, null, 2)}\n`),
  writeFile(cargoPath, nextCargo),
  writeFile(indexPath, nextIndex),
]);

console.log(`已把项目版本同步为 ${version}`);
