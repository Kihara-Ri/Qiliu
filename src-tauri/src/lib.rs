#[cfg(target_os = "android")]
mod android_context;
mod bilibili;
mod bilibili_auth;
mod cctv;
mod huya;
mod douyu;
mod douyin;
mod douyin_sign;
mod live_source;
mod huya_wup;
mod stream;

use bilibili::BilibiliClient;
use bilibili_auth::{BilibiliAuthStatus, BilibiliQrLogin, BilibiliQrPoll};
use cctv::CctvClient;
use huya::HuyaClient;
use douyu::DouyuClient;
use douyin::DouyinClient;
use stream::LiveStream;
use tauri::State;
use url::Url;

fn initialize_secure_store() -> Result<(), String> {
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    let store = apple_native_keyring_store::keychain::Store::new()
        .map_err(|error| format!("无法初始化 Apple 钥匙串：{error}"))?;

    #[cfg(target_os = "windows")]
    let store = windows_native_keyring_store::Store::new()
        .map_err(|error| format!("无法初始化 Windows 凭据管理器：{error}"))?;

    #[cfg(target_os = "android")]
    let store = {
        android_context::initialize()?;
        android_native_keyring_store::Store::new()
            .map_err(|error| format!("无法初始化 Android 安全存储：{error}"))?
    };

    #[cfg(any(
        target_os = "macos",
        target_os = "ios",
        target_os = "windows",
        target_os = "android"
    ))]
    keyring_core::set_default_store(store);

    Ok(())
}

struct LiveClients {
    huya: HuyaClient,
    bilibili: BilibiliClient,
    douyu: DouyuClient,
    douyin: DouyinClient,
    cctv: CctvClient,
}

impl LiveClients {
    fn new() -> Result<Self, String> {
        Ok(Self {
            huya: HuyaClient::new()?,
            bilibili: BilibiliClient::new()?,
            douyu: DouyuClient::new()?,
            douyin: DouyinClient::new()?,
            cctv: CctvClient::new()?,
        })
    }

    async fn resolve(
        &self,
        source: &str,
        line_index: usize,
        bitrate: u32,
    ) -> Result<LiveStream, String> {
        match source_platform(source)? {
            SourcePlatform::Huya => self.huya.resolve(source, line_index, bitrate).await,
            SourcePlatform::Bilibili => self.bilibili.resolve(source, line_index, bitrate).await,
            SourcePlatform::Douyu => self.douyu.resolve(source, line_index, bitrate).await,
            SourcePlatform::Douyin => self.douyin.resolve(source, line_index, bitrate).await,
            SourcePlatform::Cctv => self.cctv.resolve(source, line_index, bitrate).await,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SourcePlatform {
    Huya,
    Bilibili,
    Douyu,
    Douyin,
    Cctv,
}

fn source_platform(source: &str) -> Result<SourcePlatform, String> {
    let source = source.trim();
    let normalized = if source.contains("://") {
        source.to_string()
    } else {
        format!("https://{source}")
    };
    let url = Url::parse(&normalized).map_err(|_| "直播间链接格式不正确".to_string())?;
    let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
    if host == "huya.com" || host.ends_with(".huya.com") {
        return Ok(SourcePlatform::Huya);
    }
    if host == "bilibili.com" || host.ends_with(".bilibili.com") {
        return Ok(SourcePlatform::Bilibili);
    }
    if ["douyu.com", "www.douyu.com", "m.douyu.com"].contains(&host.as_str()) { return Ok(SourcePlatform::Douyu); }
    if ["live.douyin.com", "v.douyin.com"].contains(&host.as_str()) { return Ok(SourcePlatform::Douyin); }
    if ["tv.cctv.com", "www.cctv.com", "live.cctv.com", "cctv.com"].contains(&host.as_str()) { return Ok(SourcePlatform::Cctv); }
    Err("目前支持虎牙、Bilibili、斗鱼、抖音和央视频道直播间链接".to_string())
}

#[tauri::command]
async fn get_live_status(clients: State<'_, LiveClients>, source: String) -> Result<bool, String> {
    match source_platform(&source)? {
        SourcePlatform::Huya => clients.huya.is_live(&source).await,
        SourcePlatform::Bilibili => clients.bilibili.is_live(&source).await,
        SourcePlatform::Douyu => clients.douyu.is_live(&source).await,
        SourcePlatform::Douyin => clients.douyin.is_live(&source).await,
        SourcePlatform::Cctv => clients.cctv.is_live(&source).await,
    }
}

#[tauri::command]
async fn resolve_live_stream(
    clients: State<'_, LiveClients>,
    source: String,
    line_index: usize,
    bitrate: u32,
) -> Result<LiveStream, String> {
    clients.resolve(&source, line_index, bitrate).await
}

#[tauri::command]
async fn get_bilibili_auth_status(
    clients: State<'_, LiveClients>,
) -> Result<BilibiliAuthStatus, String> {
    clients.bilibili.auth_status().await
}

#[tauri::command]
async fn start_bilibili_qr_login(
    clients: State<'_, LiveClients>,
) -> Result<BilibiliQrLogin, String> {
    clients.bilibili.start_qr_login().await
}

#[tauri::command]
async fn poll_bilibili_qr_login(clients: State<'_, LiveClients>) -> Result<BilibiliQrPoll, String> {
    clients.bilibili.poll_qr_login().await
}

#[tauri::command]
async fn logout_bilibili(clients: State<'_, LiveClients>) -> Result<BilibiliAuthStatus, String> {
    clients.bilibili.logout()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let live_clients = LiveClients::new().expect("failed to initialize live-source HTTP clients");
    let builder = tauri::Builder::default().plugin(tauri_plugin_clipboard_manager::init());

    #[cfg(target_os = "macos")]
    let builder = builder.enable_macos_default_menu(false);

    builder
        .setup(|_| {
            // Android's Activity is not available before Builder::run. Secure
            // storage is optional: report its failure through account commands,
            // rather than aborting the entire player at launch.
            if let Err(error) = initialize_secure_store() {
                eprintln!("Qiliu secure store unavailable: {error}");
            }
            Ok(())
        })
        .manage(live_clients)
        .invoke_handler(tauri::generate_handler![
            resolve_live_stream,
            get_live_status,
            get_bilibili_auth_status,
            start_bilibili_qr_login,
            poll_bilibili_qr_login,
            logout_bilibili
        ])
        .run(tauri::generate_context!())
        .expect("error while running Qiliu");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires QILIU_LIVE_PROBE and access to the real live platform"]
    fn probe_live_source() {
        let source = std::env::var("QILIU_LIVE_PROBE").expect("set QILIU_LIVE_PROBE to a public live room URL");
        tauri::async_runtime::block_on(async {
            let line = std::env::var("QILIU_PROBE_LINE").ok().and_then(|s| s.parse().ok()).unwrap_or(0);
            let quality = std::env::var("QILIU_PROBE_QUALITY").ok().and_then(|s| s.parse().ok()).unwrap_or(0);
            let stream = LiveClients::new().unwrap().resolve(&source, line, quality).await.unwrap();
            println!("platform={} room={} live={} quality={} lines={}", stream.platform, stream.room_id, stream.is_live, stream.quality_label, stream.line_count);
            if let Some(url) = stream.url {
                let http = live_source::client(&stream.source_url).unwrap();
                let mut response = http.get(url).header("Origin", "http://tauri.localhost").send().await.unwrap().error_for_status().unwrap();
                println!("media type={:?} cors={:?}", response.headers().get("content-type"), response.headers().get("access-control-allow-origin"));
                let first = response.chunk().await.unwrap().expect("stream must return bytes");
                assert!(!first.is_empty());
                if first.len() >= 7 {
                    assert!(first.starts_with(b"FLV") || first.starts_with(b"#EXTM3U"), "expected FLV or HLS media, not an HTML error page");
                }
                println!("received {} initial bytes", first.len());
            }
        });
    }

    #[test]
    fn routes_supported_live_sources() {
        assert_eq!(
            source_platform("https://www.huya.com/196645").unwrap(),
            SourcePlatform::Huya
        );
        assert_eq!(
            source_platform("https://live.bilibili.com/5050").unwrap(),
            SourcePlatform::Bilibili
        );
        assert_eq!(source_platform("https://www.douyu.com/9999").unwrap(), SourcePlatform::Douyu);
        assert_eq!(source_platform("https://live.douyin.com/123456").unwrap(), SourcePlatform::Douyin);
        assert_eq!(source_platform("https://v.douyin.com/AbCd/").unwrap(), SourcePlatform::Douyin);
        assert_eq!(
            source_platform("https://tv.cctv.com/live/cctv1/").unwrap(),
            SourcePlatform::Cctv
        );
        assert!(source_platform("https://example.com/5050").is_err());
    }
}
