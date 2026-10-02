//! 体验补丁通道：拉取、验签、存储仓库 `patches/` 目录分发的签名前端模块包。
//!
//! 补丁只覆盖前端层的小改动，写入应用数据目录（不触碰安装包本体，macOS
//! bundle 签名保持完整，用户数据不受影响），再通过 `qiliu-patch://` 自定义
//! 协议按 sha256 校验后提供给页面动态加载。索引与文件都必须通过 ed25519
//! 验签，公钥内嵌在本文件中，构成从已安装应用到补丁内容的完整信任链。

use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::borrow::Cow;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::http;
use tauri::{Manager, State};
use url::Url;

/// 补丁签名公钥（ed25519 原始 32 字节，hex）。
/// 由 `node scripts/make-patch.mjs --generate-key` 生成；更换密钥需发新版本。
const PATCH_PUBKEY_HEX: &str =
    "a88099936476b1219b21f25a26c6679af3d82f24a4caab41e3333a8583199ef3";

/// 补丁索引的获取端点，按顺序尝试；jsdelivr 作为国内可达性备用。
const INDEX_ENDPOINTS: &[&str] = &[
    "https://raw.githubusercontent.com/Kihara-Ri/Qiliu/main/patches/index.json",
    "https://cdn.jsdelivr.net/gh/Kihara-Ri/Qiliu@main/patches/index.json",
];

const SCHEMA_VERSION: u32 = 1;
const MAX_INDEX_BYTES: usize = 256 * 1024;
const MAX_FILE_BYTES: usize = 2 * 1024 * 1024;
const MAX_TOTAL_FILE_BYTES: usize = 8 * 1024 * 1024;
/// 入口模块连续加载失败的自动停用阈值。
const FAILURE_DISABLE_THRESHOLD: u32 = 2;

const STATE_FILE: &str = "state.json";
const ENTRY_FILE: &str = "entry.js";

#[derive(Deserialize)]
struct PatchEnvelope {
    schema: u32,
    payload: serde_json::Value,
    signature: String,
}

#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PatchPayload {
    pub schema: u32,
    pub sequence: u64,
    pub min_app_version: String,
    pub max_app_version: Option<String>,
    pub notes: String,
    pub base_url: String,
    pub files: BTreeMap<String, String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
struct ActivePatch {
    sequence: u64,
    notes: String,
    files: BTreeMap<String, String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
struct PatchHubState {
    enabled: bool,
    auto_disabled: bool,
    active: Option<ActivePatch>,
    failures: u32,
    dismissed_sequence: Option<u64>,
    last_check: Option<u64>,
    last_error: Option<String>,
}

impl Default for PatchHubState {
    fn default() -> Self {
        Self {
            enabled: true,
            auto_disabled: false,
            active: None,
            failures: 0,
            dismissed_sequence: None,
            last_check: None,
            last_error: None,
        }
    }
}

/// 一次补丁检查的结果，供前端决定是否立即加载新补丁。
#[derive(Serialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum CheckOutcome {
    /// 没有更新的补丁（基线或已应用过）。
    UpToDate,
    /// 成功安装并激活了新补丁。
    Applied { sequence: u64 },
    /// 索引里的补丁不适用于当前应用版本。
    Inapplicable { reason: String },
    /// 补丁被用户停用或已回滚，跳过。
    Skipped,
    /// 网络、验签等环节失败（原因记录在 state.last_error）。
    Failed { reason: String },
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PatchStateView {
    pub enabled: bool,
    pub auto_disabled: bool,
    pub active_version: Option<u64>,
    pub notes: String,
    pub failures: u32,
    pub last_check: Option<u64>,
    pub last_error: Option<String>,
    pub entry_url: Option<String>,
}

/// `check_patches` 命令的返回：命令本身不因检查失败而报错，失败通过 outcome 传递。
#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PatchCheckResult {
    pub state: PatchStateView,
    pub outcome: CheckOutcome,
}

/// 由 Tauri 管理的补丁中枢状态；磁盘落在应用数据目录的 `patches/` 下。
pub struct PatchHub {
    root: PathBuf,
    app_version: String,
    verifying_key: VerifyingKey,
    inner: Mutex<PatchHubState>,
}

impl PatchHub {
    pub fn initialize(app: &tauri::AppHandle, app_version: String) -> Self {
        let root = app
            .path()
            .app_data_dir()
            .map(|dir| dir.join("patches"))
            .unwrap_or_else(|error| {
                eprintln!("Qiliu patch hub storage unavailable: {error}");
                std::env::temp_dir().join("qiliu-patches-unavailable")
            });
        Self::new(root, app_version)
    }

    pub fn new(root: PathBuf, app_version: String) -> Self {
        // 公钥是内嵌常量，损坏即代码错误，直接 panic 暴露。
        let key = verifying_key().expect("embedded patch pubkey is valid");
        Self::with_verifying_key(root, app_version, key)
    }

    #[cfg(test)]
    pub fn with_test_verifying_key(root: PathBuf, app_version: String, key: VerifyingKey) -> Self {
        Self::with_verifying_key(root, app_version, key)
    }

    fn with_verifying_key(
        root: PathBuf,
        app_version: String,
        verifying_key: VerifyingKey,
    ) -> Self {
        let inner = load_state(&root).unwrap_or_default();
        Self {
            root,
            app_version,
            verifying_key,
            inner: Mutex::new(inner),
        }
    }

    pub fn view(&self) -> PatchStateView {
        let inner = lock(&self.inner);
        let servable = inner.enabled && !inner.auto_disabled && inner.active.is_some();
        PatchStateView {
            enabled: inner.enabled,
            auto_disabled: inner.auto_disabled,
            active_version: inner.active.as_ref().map(|patch| patch.sequence),
            notes: inner.active.as_ref().map(|patch| patch.notes.clone()).unwrap_or_default(),
            failures: inner.failures,
            last_check: inner.last_check,
            last_error: inner.last_error.clone(),
            entry_url: servable.then(|| {
                inner
                    .active
                    .as_ref()
                    .map(|patch| entry_url(patch.sequence))
                    .unwrap_or_default()
            }),
        }
    }

    /// 拉取远端索引并按需安装；网络等错误记录到 last_error 并返回 Err。
    pub async fn run_check(&self) -> Result<CheckOutcome, String> {
        let bytes = fetch_index_bytes().await?;
        self.apply_index_bytes(bytes, &PatchFetcher::network()?).await
    }

    /// 校验并应用一份索引内容（测试可直接注入 fixture 字节）。
    pub async fn apply_index_bytes(
        &self,
        bytes: Vec<u8>,
        fetcher: &PatchFetcher,
    ) -> Result<CheckOutcome, String> {
        let payload = verify_envelope(&bytes, &self.verifying_key)?;

        {
            let mut inner = lock(&self.inner);
            inner.last_check = Some(unix_now());
            inner.last_error = None;
        }

        if !payload_applicable(&payload, &self.app_version) {
            let reason = format!(
                "补丁 v{} 不适用于当前应用版本 {}/{}",
                payload.sequence,
                self.app_version,
                payload.min_app_version
            );
            let mut inner = lock(&self.inner);
            inner.last_error = Some(reason.clone());
            return Ok(CheckOutcome::Inapplicable { reason });
        }

        let (enabled, active_sequence, dismissed) = {
            let inner = lock(&self.inner);
            (
                inner.enabled && !inner.auto_disabled,
                inner.active.as_ref().map(|patch| patch.sequence),
                inner.dismissed_sequence,
            )
        };
        if !enabled {
            return Ok(CheckOutcome::Skipped);
        }
        let known = active_sequence.unwrap_or(0).max(dismissed.unwrap_or(0));
        if payload.sequence <= known {
            return Ok(CheckOutcome::UpToDate);
        }

        self.install_payload(&payload, fetcher).await
    }

    async fn install_payload(
        &self,
        payload: &PatchPayload,
        fetcher: &PatchFetcher,
    ) -> Result<CheckOutcome, String> {
        if payload.files.is_empty() {
            // sequence 0 的空基线：只视为“没有补丁”，不落盘。
            let mut inner = lock(&self.inner);
            inner.active = None;
            inner.failures = 0;
            inner.auto_disabled = false;
            save_state(&self.root, &inner);
            return Ok(CheckOutcome::UpToDate);
        }

        let destination = self.version_dir(payload.sequence);
        let staging = self.root.join(format!(".staging-{}", payload.sequence));
        if staging.exists() {
            std::fs::remove_dir_all(&staging).map_err(|error| format!("清理补丁暂存目录失败：{error}"))?;
        }
        std::fs::create_dir_all(&staging).map_err(|error| format!("创建补丁暂存目录失败：{error}"))?;

        let mut total = 0usize;
        for (name, expected_hash) in &payload.files {
            let urls = download_candidates(&payload.base_url, name)?;
            let bytes = fetcher.fetch(&urls).await?;
            if bytes.len() > MAX_FILE_BYTES {
                let _ = std::fs::remove_dir_all(&staging);
                return Err(format!("补丁文件 {name} 超出单文件大小限制"));
            }
            total += bytes.len();
            if total > MAX_TOTAL_FILE_BYTES {
                let _ = std::fs::remove_dir_all(&staging);
                return Err("补丁内容超出总大小限制".to_string());
            }
            if sha256_hex(&bytes) != *expected_hash {
                let _ = std::fs::remove_dir_all(&staging);
                return Err(format!("补丁文件 {name} 的 sha256 校验失败"));
            }
            std::fs::write(staging.join(name), &bytes)
                .map_err(|error| format!("写入补丁文件 {name} 失败：{error}"))?;
        }

        // 索引本身也落盘，作为提供文件时的第二重校验依据。
        let manifest = serde_json::to_vec(payload).map_err(|error| format!("序列化补丁清单失败：{error}"))?;
        std::fs::write(staging.join("manifest.json"), &manifest)
            .map_err(|error| format!("写入补丁清单失败：{error}"))?;

        if destination.exists() {
            std::fs::remove_dir_all(&destination)
                .map_err(|error| format!("替换旧补丁目录失败：{error}"))?;
        }
        std::fs::rename(&staging, &destination).map_err(|error| format!("激活补丁目录失败：{error}"))?;
        prune_versions(&self.root, payload.sequence);

        {
            let mut inner = lock(&self.inner);
            inner.active = Some(ActivePatch {
                sequence: payload.sequence,
                notes: payload.notes.clone(),
                files: payload.files.clone(),
            });
            inner.failures = 0;
            inner.auto_disabled = false;
            save_state(&self.root, &inner);
        }

        Ok(CheckOutcome::Applied {
            sequence: payload.sequence,
        })
    }

    /// 按激活补丁的 sha256 清单提供文件内容；任何不一致都拒绝提供。
    pub fn serve_file(&self, name: &str) -> Option<(Vec<u8>, &'static str)> {
        let inner = lock(&self.inner);
        if !inner.enabled || inner.auto_disabled {
            return None;
        }
        let active = inner.active.as_ref()?;
        if !is_safe_file_name(name) {
            return None;
        }
        let expected_hash = active.files.get(name)?;
        let bytes = std::fs::read(self.version_dir(active.sequence).join(name)).ok()?;
        if sha256_hex(&bytes) != *expected_hash {
            return None;
        }
        Some((bytes, mime_of(name)))
    }

    pub fn record_health(&self, ok: bool, message: Option<String>) {
        let mut inner = lock(&self.inner);
        if ok {
            inner.failures = 0;
            inner.auto_disabled = false;
        } else {
            inner.failures = inner.failures.saturating_add(1);
            if let Some(message) = message {
                inner.last_error = Some(format!("补丁加载失败：{message}"));
            }
            if inner.failures >= FAILURE_DISABLE_THRESHOLD {
                inner.auto_disabled = true;
            }
        }
        save_state(&self.root, &inner);
    }

    fn record_check_error(&self, error: String) {
        let mut inner = lock(&self.inner);
        inner.last_check = Some(unix_now());
        inner.last_error = Some(error);
        save_state(&self.root, &inner);
    }

    pub fn set_enabled(&self, enabled: bool) {
        let mut inner = lock(&self.inner);
        inner.enabled = enabled;
        if enabled {
            // 重新启用视为一次重试，清空失败与自动停用状态。
            inner.failures = 0;
            inner.auto_disabled = false;
        }
        save_state(&self.root, &inner);
    }

    pub fn rollback(&self) {
        let mut inner = lock(&self.inner);
        if let Some(active) = inner.active.take() {
            inner.dismissed_sequence = Some(active.sequence);
        }
        inner.failures = 0;
        inner.auto_disabled = false;
        save_state(&self.root, &inner);
    }

    fn version_dir(&self, sequence: u64) -> PathBuf {
        self.root.join(sequence.to_string())
    }
}

/// 文件获取途径：线上按候选地址依次下载；测试注入固定字节。
pub enum PatchFetcher {
    Network { client: reqwest::Client },
    #[cfg(test)]
    Fixture(BTreeMap<String, Vec<u8>>),
}

impl PatchFetcher {
    fn network() -> Result<Self, String> {
        let client = reqwest::Client::builder()
            .user_agent(concat!("QiliuPatchHub/", env!("CARGO_PKG_VERSION")))
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|error| format!("初始化补丁下载客户端失败：{error}"))?;
        Ok(Self::Network { client })
    }

    async fn fetch(&self, urls: &[String]) -> Result<Vec<u8>, String> {
        match self {
            Self::Network { client } => {
                let mut last = "没有可用的下载地址".to_string();
                for url in urls {
                    match client
                        .get(url)
                        .timeout(Duration::from_secs(30))
                        .send()
                        .await
                    {
                        Ok(response) if response.status().is_success() => {
                            let bytes = response
                                .bytes()
                                .await
                                .map_err(|error| format!("下载补丁文件失败：{error}"))?;
                            if bytes.len() > MAX_FILE_BYTES {
                                return Err("补丁文件超出单文件大小限制".to_string());
                            }
                            return Ok(bytes.to_vec());
                        }
                        Ok(response) => last = format!("HTTP {}", response.status()),
                        Err(error) => last = error.to_string(),
                    }
                }
                Err(format!("下载补丁文件失败：{last}"))
            }
            #[cfg(test)]
            Self::Fixture(map) => {
                // fixture 以文件名为键；从首选地址的末段还原文件名。
                let name = urls
                    .first()
                    .ok_or("补丁文件缺少下载地址")?
                    .rsplit('/')
                    .find(|segment| !segment.is_empty())
                    .unwrap_or_default();
                map.get(name)
                    .cloned()
                    .ok_or_else(|| format!("fixture 缺少补丁文件 {name}"))
            }
        }
    }
}

/// 自定义协议入口：只提供激活补丁清单内、sha256 一致的文件。
pub fn handle_protocol_request<R: tauri::Runtime>(
    ctx: tauri::UriSchemeContext<'_, R>,
    request: http::Request<Vec<u8>>,
) -> http::Response<Cow<'static, [u8]>> {
    let hub = ctx.app_handle().state::<PatchHub>();
    let name = request
        .uri()
        .path()
        .rsplit('/')
        .find(|segment| !segment.is_empty())
        .unwrap_or_default()
        .to_string();
    match hub.serve_file(&name) {
        Some((bytes, mime)) => http::Response::builder()
            .header(http::header::CONTENT_TYPE, mime)
            // 页面与补丁协议来源不同，模块加载按 CORS 处理。
            .header("Access-Control-Allow-Origin", "*")
            .header("Cache-Control", "no-store")
            .body(Cow::Owned(bytes))
            .expect("static patch protocol response"),
        None => http::Response::builder()
            .status(http::StatusCode::NOT_FOUND)
            .header(http::header::CONTENT_TYPE, "text/plain; charset=utf-8")
            .body(Cow::Borrowed(b"patch not available".as_slice()))
            .expect("static patch protocol response"),
    }
}

fn entry_url(sequence: u64) -> String {
    // Windows/Android 上自定义协议映射为 http://<scheme>.localhost/。
    if cfg!(any(target_os = "windows", target_os = "android")) {
        format!("http://qiliu-patch.localhost/{ENTRY_FILE}?v={sequence}")
    } else {
        format!("qiliu-patch://localhost/{ENTRY_FILE}?v={sequence}")
    }
}

fn verifying_key() -> Result<VerifyingKey, String> {
    let bytes = hex_decode_32(PATCH_PUBKEY_HEX)?;
    VerifyingKey::from_bytes(&bytes).map_err(|error| format!("补丁公钥不可用：{error}"))
}

/// 校验索引：签名覆盖 payload 的规范化 JSON（键递归排序、紧凑序列化），
/// 与 scripts/make-patch.mjs 的 canonicalJson 逐字节一致。
pub fn verify_envelope(bytes: &[u8], key: &VerifyingKey) -> Result<PatchPayload, String> {
    if bytes.len() > MAX_INDEX_BYTES {
        return Err("补丁索引超出大小限制".to_string());
    }
    let envelope: PatchEnvelope =
        serde_json::from_slice(bytes).map_err(|error| format!("补丁索引格式不正确：{error}"))?;
    if envelope.schema != SCHEMA_VERSION {
        return Err(format!("补丁索引 schema 不支持：{}", envelope.schema));
    }
    let canonical = serde_json::to_vec(&envelope.payload)
        .map_err(|error| format!("规范化补丁索引失败：{error}"))?;
    let signature_bytes = hex_decode_64(&envelope.signature)?;
    let signature =
        Signature::from_slice(&signature_bytes).map_err(|_| "补丁签名格式不正确".to_string())?;
    key.verify(&canonical, &signature)
        .map_err(|_| "补丁签名校验失败：已拒绝".to_string())?;
    serde_json::from_value(envelope.payload).map_err(|error| format!("补丁内容不完整：{error}"))
}

/// 版本门槛与文件清单校验。基线（sequence 0）允许空清单。
pub fn payload_applicable(payload: &PatchPayload, app_version: &str) -> bool {
    if payload.schema != SCHEMA_VERSION {
        return false;
    }
    let Some(current) = parse_semver(app_version) else {
        return false;
    };
    let Some(minimum) = parse_semver(&payload.min_app_version) else {
        return false;
    };
    if current < minimum {
        return false;
    }
    if let Some(maximum) = payload.max_app_version.as_deref().and_then(parse_semver) {
        if current > maximum {
            return false;
        }
    }
    if payload.sequence == 0 {
        return payload.files.is_empty();
    }
    if !payload.files.contains_key(ENTRY_FILE) {
        return false;
    }
    if !payload.files.keys().all(|name| is_safe_file_name(name)) {
        return false;
    }
    let Some(url) = Url::parse(&payload.base_url).ok() else {
        return false;
    };
    if url.scheme() != "https" {
        return false;
    }
    matches!(
        url.host_str(),
        Some("raw.githubusercontent.com") | Some("cdn.jsdelivr.net")
    )
}

/// 解析 `1.5.0`、`v1.5`、`1.5.0-beta.1` 形式的版本号。
pub fn parse_semver(value: &str) -> Option<(u64, u64, u64)> {
    let value = value.trim().trim_start_matches('v');
    // 预发布/构建段可能带点号，先整体剥掉再按三段解析。
    let core = value.split_once(['-', '+']).map(|(core, _)| core).unwrap_or(value);
    let mut parts = core.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next().unwrap_or("0").parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

/// candidate 是否严格新于 current（Android 发布检查使用；桌面端由更新器插件自比）。
#[cfg_attr(desktop, allow(dead_code))]
pub fn version_newer(candidate: &str, current: &str) -> bool {
    match (parse_semver(candidate), parse_semver(current)) {
        (Some(candidate), Some(current)) => candidate > current,
        _ => false,
    }
}

pub fn is_safe_file_name(name: &str) -> bool {
    let mut characters = name.chars();
    match characters.next() {
        Some(first) if first.is_ascii_alphanumeric() => {}
        _ => return false,
    }
    name.len() <= 64
        && characters.all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_')
        })
        && !name.contains("..")
}

fn download_candidates(base_url: &str, name: &str) -> Result<Vec<String>, String> {
    let base = Url::parse(base_url).map_err(|_| "补丁 baseUrl 不合法".to_string())?;
    let primary = base.join(name).map_err(|_| "补丁文件地址不合法".to_string())?;
    let mut candidates = vec![primary.to_string()];
    if base.host_str() == Some("raw.githubusercontent.com") {
        // https://raw.githubusercontent.com/{owner}/{repo}/{ref}/{rest}
        let segments: Vec<&str> = base.path().trim_matches('/').splitn(4, '/').collect();
        if segments.len() == 4 {
            let mirror = format!(
                "https://cdn.jsdelivr.net/gh/{}/{}@{}/{}",
                segments[0],
                segments[1],
                segments[2],
                ensure_trailing_slash(segments[3])
            );
            if let Ok(mirror) = Url::parse(&mirror).and_then(|url| url.join(name)) {
                candidates.push(mirror.to_string());
            }
        }
    }
    Ok(candidates)
}

fn ensure_trailing_slash(value: &str) -> String {
    if value.ends_with('/') {
        value.to_string()
    } else {
        format!("{value}/")
    }
}

async fn fetch_index_bytes() -> Result<Vec<u8>, String> {
    let client = reqwest::Client::builder()
        .user_agent(concat!("QiliuPatchHub/", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|error| format!("初始化补丁检查客户端失败：{error}"))?;
    let mut last = "没有可用的检查地址".to_string();
    for endpoint in INDEX_ENDPOINTS {
        match client.get(*endpoint).send().await {
            Ok(response) if response.status().is_success() => {
                let bytes = response
                    .bytes()
                    .await
                    .map_err(|error| format!("读取补丁索引失败：{error}"))?;
                if bytes.len() > MAX_INDEX_BYTES {
                    return Err("补丁索引超出大小限制".to_string());
                }
                return Ok(bytes.to_vec());
            }
            Ok(response) => last = format!("HTTP {}", response.status()),
            Err(error) => last = error.to_string(),
        }
    }
    Err(format!("获取补丁索引失败：{last}"))
}

fn load_state(root: &Path) -> Option<PatchHubState> {
    serde_json::from_slice(&std::fs::read(root.join(STATE_FILE)).ok()?).ok()
}

fn save_state(root: &Path, state: &PatchHubState) {
    if let Err(error) = std::fs::create_dir_all(root) {
        eprintln!("Qiliu patch state directory unavailable: {error}");
        return;
    }
    let target = root.join(STATE_FILE);
    let temporary = root.join(format!("{STATE_FILE}.tmp"));
    let serialized = match serde_json::to_vec(state) {
        Ok(serialized) => serialized,
        Err(error) => {
            eprintln!("Qiliu patch state serialize failed: {error}");
            return;
        }
    };
    if let Err(error) = std::fs::write(&temporary, &serialized) {
        eprintln!("Qiliu patch state write failed: {error}");
        return;
    }
    if let Err(error) = std::fs::rename(&temporary, &target) {
        eprintln!("Qiliu patch state commit failed: {error}");
    }
}

fn prune_versions(root: &Path, keep: u64) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if let Some(directory) = name.strip_prefix(".staging-") {
            if directory.parse::<u64>() != Ok(keep) {
                let _ = std::fs::remove_dir_all(entry.path());
            }
            continue;
        }
        if name.parse::<u64>() == Ok(keep) {
            continue;
        }
        if name.parse::<u64>().is_ok() {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut hex = String::with_capacity(64);
    for byte in digest {
        hex.push_str(&format!("{byte:02x}"));
    }
    hex
}

fn hex_decode_32(value: &str) -> Result<[u8; 32], String> {
    let bytes = hex_decode(value)?;
    bytes
        .try_into()
        .map_err(|_| "补丁公钥长度不正确".to_string())
}

fn hex_decode_64(value: &str) -> Result<[u8; 64], String> {
    let bytes = hex_decode(value)?;
    bytes
        .try_into()
        .map_err(|_| "补丁签名长度不正确".to_string())
}

fn hex_decode(value: &str) -> Result<Vec<u8>, String> {
    let value = value.trim();
    if value.len() % 2 != 0 || !value.chars().all(|character| character.is_ascii_hexdigit()) {
        return Err("hex 格式不正确".to_string());
    }
    Ok((0..value.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&value[index..index + 2], 16))
        .collect::<Result<_, _>>()
        .map_err(|_| "hex 解码失败".to_string())?)
}

fn mime_of(name: &str) -> &'static str {
    if name.ends_with(".js") || name.ends_with(".mjs") {
        "text/javascript; charset=utf-8"
    } else if name.ends_with(".css") {
        "text/css; charset=utf-8"
    } else if name.ends_with(".json") {
        "application/json"
    } else if name.ends_with(".svg") {
        "image/svg+xml"
    } else {
        "application/octet-stream"
    }
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

fn lock(state: &Mutex<PatchHubState>) -> std::sync::MutexGuard<'_, PatchHubState> {
    state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[tauri::command]
pub fn get_patch_state(hub: State<'_, PatchHub>) -> Result<PatchStateView, String> {
    Ok(hub.view())
}

#[tauri::command]
pub async fn check_patches(app: tauri::AppHandle) -> Result<PatchCheckResult, String> {
    // 异步命令内经 AppHandle 取状态，避免 State<'_, T> 借用跨越 await。
    let hub = app.state::<PatchHub>();
    let outcome = match hub.run_check().await {
        Ok(outcome) => outcome,
        // 网络、验签等失败不作为命令错误抛出：记录到 last_error，由视图呈现。
        Err(error) => {
            hub.record_check_error(error.clone());
            CheckOutcome::Failed { reason: error }
        }
    };
    Ok(PatchCheckResult {
        state: hub.view(),
        outcome,
    })
}

#[tauri::command]
pub fn set_patch_enabled(hub: State<'_, PatchHub>, enabled: bool) -> Result<PatchStateView, String> {
    hub.set_enabled(enabled);
    Ok(hub.view())
}

#[tauri::command]
pub fn rollback_patch(hub: State<'_, PatchHub>) -> Result<PatchStateView, String> {
    hub.rollback();
    Ok(hub.view())
}

#[tauri::command]
pub fn report_patch_health(
    hub: State<'_, PatchHub>,
    ok: bool,
    message: Option<String>,
) -> Result<PatchStateView, String> {
    hub.record_health(ok, message);
    Ok(hub.view())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::SigningKey;

    const FIXTURE_PUBLIC_KEY_HEX: &str = "8a88e3dd7409f195fd52db2d3cba5d72ca6709bf1d94121bf3748801b40f6f5c";

    fn fixture() -> serde_json::Value {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/patch-index-fixture.json");
        serde_json::from_str(&std::fs::read_to_string(path).expect("fixture file"))
            .expect("fixture json")
    }

    fn fixture_envelope_bytes() -> Vec<u8> {
        serde_json::to_vec(&fixture()["envelope"]).expect("envelope json")
    }

    fn fixture_files() -> BTreeMap<String, Vec<u8>> {
        let document = fixture();
        let files = document["files"].as_object().expect("fixture files");
        files
            .iter()
            .map(|(name, value)| {
                (
                    name.clone(),
                    general_purpose_standard_decode(value.as_str().expect("base64")),
                )
            })
            .collect()
    }

    fn general_purpose_standard_decode(value: &str) -> Vec<u8> {
        // base64 crate 已在依赖中，但为保持测试自包含，这里直接使用它。
        use base64::Engine as _;
        base64::engine::general_purpose::STANDARD
            .decode(value)
            .expect("fixture base64")
    }

    fn temp_root(label: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "qiliu-patch-tests-{}-{label}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("temp dir");
        root
    }

    fn fixture_verifying_key() -> VerifyingKey {
        let bytes = hex_decode_32(FIXTURE_PUBLIC_KEY_HEX).expect("fixture pubkey");
        VerifyingKey::from_bytes(&bytes).expect("fixture pubkey")
    }

    #[test]
    fn verifies_fixture_signed_by_node_script() {
        let payload = verify_envelope(&fixture_envelope_bytes(), &fixture_verifying_key())
            .expect("fixture verifies");
        assert_eq!(payload.sequence, 7);
        assert!(payload.files.contains_key("entry.js"));
    }

    #[test]
    fn rejects_tampered_payload() {
        let mut value: serde_json::Value =
            serde_json::from_slice(&fixture_envelope_bytes()).expect("envelope");
        value["payload"]["sequence"] = serde_json::json!(999);
        let tampered = serde_json::to_vec(&value).expect("tampered");
        assert!(verify_envelope(&tampered, &fixture_verifying_key()).is_err());
    }

    #[test]
    fn rejects_wrong_key_and_bad_hex() {
        let unrelated = SigningKey::from_bytes(&[7u8; 32]);
        assert!(
            verify_envelope(&fixture_envelope_bytes(), &unrelated.verifying_key()).is_err()
        );
        let mut value: serde_json::Value =
            serde_json::from_slice(&fixture_envelope_bytes()).expect("envelope");
        value["signature"] = serde_json::json!("zz");
        let malformed = serde_json::to_vec(&value).expect("malformed");
        assert!(verify_envelope(&malformed, &fixture_verifying_key()).is_err());
    }

    #[test]
    fn canonical_bytes_match_sorted_compact_form() {
        // serde_json::Value 的序列化必须等于脚本端“递归排序 + 紧凑”的形态，
        // 否则跨实现签名必然失败；fixture 由 Node 脚本生成，携带它算出的规范形态。
        let mut value: serde_json::Value =
            serde_json::from_slice(&fixture_envelope_bytes()).expect("envelope");
        let payload = value["payload"].take();
        let canonical = serde_json::to_vec(&payload).expect("canonical");
        let document = fixture();
        let expected = document["canonicalJson"].as_str().expect("canonicalJson");
        assert_eq!(canonical.as_slice(), expected.as_bytes());
    }

    #[test]
    fn parses_semver_and_compares() {
        assert_eq!(parse_semver("1.5.0"), Some((1, 5, 0)));
        assert_eq!(parse_semver("v1.5"), Some((1, 5, 0)));
        assert_eq!(parse_semver("1.5.0-beta.1"), Some((1, 5, 0)));
        assert_eq!(parse_semver("abc"), None);
        assert_eq!(parse_semver("1.5.0.1"), None);
        assert!(version_newer("1.6.0", "1.5.9"));
        assert!(!version_newer("1.5.0", "1.5.0"));
        assert!(!version_newer("1.4.9", "1.5.0"));
    }

    #[test]
    fn validates_file_names() {
        assert!(is_safe_file_name("entry.js"));
        assert!(is_safe_file_name("module-2.mjs"));
        assert!(!is_safe_file_name("../escape"));
        assert!(!is_safe_file_name("a/b"));
        assert!(!is_safe_file_name(""));
        assert!(!is_safe_file_name(".hidden"));
    }

    #[test]
    fn payload_applicability_gates() {
        let payload = verify_envelope(&fixture_envelope_bytes(), &fixture_verifying_key()).unwrap();
        assert!(payload_applicable(&payload, "1.5.0"));
        assert!(payload_applicable(&payload, "2.0.0"));
        assert!(!payload_applicable(&payload, "0.9.9"));

        let mut http_base = payload.clone();
        http_base.base_url = "http://example.com/patches/".into();
        assert!(!payload_applicable(&http_base, "1.5.0"));

        let mut foreign_host = payload.clone();
        foreign_host.base_url = "https://evil.example.com/patches/".into();
        assert!(!payload_applicable(&foreign_host, "1.5.0"));

        let mut capped = payload.clone();
        capped.max_app_version = Some("1.5.0".into());
        assert!(payload_applicable(&capped, "1.5.0"));
        assert!(!payload_applicable(&capped, "1.6.0"));
    }

    #[test]
    fn builds_download_candidates_with_mirror() {
        let candidates =
            download_candidates("https://raw.githubusercontent.com/Kihara-Ri/Qiliu/main/patches/3/", "entry.js")
                .unwrap();
        assert_eq!(
            candidates[0],
            "https://raw.githubusercontent.com/Kihara-Ri/Qiliu/main/patches/3/entry.js"
        );
        assert_eq!(
            candidates[1],
            "https://cdn.jsdelivr.net/gh/Kihara-Ri/Qiliu@main/patches/3/entry.js"
        );
        let plain =
            download_candidates("https://cdn.jsdelivr.net/gh/Kihara-Ri/Qiliu@main/patches/3/", "entry.js")
                .unwrap();
        assert_eq!(plain.len(), 1);
    }

    #[tokio::test]
    async fn installs_fixture_and_serves_verified_entry() {
        let root = temp_root("install");
        let hub = PatchHub::with_test_verifying_key(root.clone(), "1.5.0".into(), fixture_verifying_key());
        let fetcher = PatchFetcher::Fixture(fixture_files());

        let outcome = hub
            .apply_index_bytes(fixture_envelope_bytes(), &fetcher)
            .await
            .expect("install");
        assert_eq!(outcome, CheckOutcome::Applied { sequence: 7 });

        let view = hub.view();
        assert_eq!(view.active_version, Some(7));
        assert!(view.entry_url.is_some());

        let (bytes, mime) = hub.serve_file("entry.js").expect("served entry");
        assert_eq!(mime, "text/javascript; charset=utf-8");
        assert!(String::from_utf8(bytes).unwrap().contains("fixture-applied"));
        assert!(hub.serve_file("manifest.json").is_none());
        assert!(hub.serve_file("missing.js").is_none());

        // 篡改磁盘文件后按哈希拒绝提供。
        let target = root.join("7").join("entry.js");
        std::fs::write(&target, b"tampered").unwrap();
        assert!(hub.serve_file("entry.js").is_none());
    }

    #[tokio::test]
    async fn up_to_date_when_sequence_not_newer() {
        let root = temp_root("uptodate");
        let hub = PatchHub::with_test_verifying_key(root, "1.5.0".into(), fixture_verifying_key());
        let fetcher = PatchFetcher::Fixture(fixture_files());
        hub.apply_index_bytes(fixture_envelope_bytes(), &fetcher)
            .await
            .expect("install");
        let second = hub
            .apply_index_bytes(fixture_envelope_bytes(), &fetcher)
            .await
            .expect("second check");
        assert_eq!(second, CheckOutcome::UpToDate);
    }

    #[tokio::test]
    async fn rollback_dismisses_active_sequence() {
        let root = temp_root("rollback");
        let hub = PatchHub::with_test_verifying_key(root, "1.5.0".into(), fixture_verifying_key());
        let fetcher = PatchFetcher::Fixture(fixture_files());
        hub.apply_index_bytes(fixture_envelope_bytes(), &fetcher)
            .await
            .expect("install");
        hub.rollback();
        let view = hub.view();
        assert_eq!(view.active_version, None);
        assert!(view.entry_url.is_none());
        // 回滚后同一版本不会自动回来（被 dismissed 挡住，不算新补丁）。
        let again = hub
            .apply_index_bytes(fixture_envelope_bytes(), &fetcher)
            .await
            .expect("recheck");
        assert_eq!(again, CheckOutcome::UpToDate);
    }

    #[tokio::test]
    async fn health_failures_auto_disable_and_recover() {
        let root = temp_root("health");
        let hub = PatchHub::with_test_verifying_key(root, "1.5.0".into(), fixture_verifying_key());
        hub.record_health(false, Some("import failed".into()));
        assert!(!hub.view().auto_disabled);
        hub.record_health(false, None);
        let view = hub.view();
        assert!(view.auto_disabled);
        assert!(view.entry_url.is_none());
        assert!(hub.serve_file("entry.js").is_none());

        hub.set_enabled(true);
        assert!(!hub.view().auto_disabled);
        hub.record_health(true, None);
        assert_eq!(hub.view().failures, 0);
    }

    #[tokio::test]
    async fn respects_enabled_toggle() {
        let root = temp_root("toggle");
        let hub = PatchHub::with_test_verifying_key(root, "1.5.0".into(), fixture_verifying_key());
        hub.set_enabled(false);
        let fetcher = PatchFetcher::Fixture(fixture_files());
        let outcome = hub
            .apply_index_bytes(fixture_envelope_bytes(), &fetcher)
            .await
            .expect("check");
        assert_eq!(outcome, CheckOutcome::Skipped);
        assert_eq!(hub.view().active_version, None);
    }

    #[tokio::test]
    async fn inapplicable_when_app_too_old() {
        let root = temp_root("old");
        let hub = PatchHub::with_test_verifying_key(root, "0.9.0".into(), fixture_verifying_key());
        let fetcher = PatchFetcher::Fixture(fixture_files());
        let outcome = hub
            .apply_index_bytes(fixture_envelope_bytes(), &fetcher)
            .await
            .expect("check");
        assert!(matches!(outcome, CheckOutcome::Inapplicable { .. }));
        assert!(hub.view().last_error.is_some());
    }

    #[test]
    fn entry_url_matches_platform_form() {
        let url = entry_url(7);
        #[cfg(any(target_os = "windows", target_os = "android"))]
        assert_eq!(url, "http://qiliu-patch.localhost/entry.js?v=7");
        #[cfg(not(any(target_os = "windows", target_os = "android")))]
        assert_eq!(url, "qiliu-patch://localhost/entry.js?v=7");
    }
}
