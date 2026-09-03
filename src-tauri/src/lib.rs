mod bilibili;
mod bilibili_auth;
mod huya;
mod huya_wup;
mod stream;

use bilibili::BilibiliClient;
use bilibili_auth::{BilibiliAuthStatus, BilibiliQrLogin, BilibiliQrPoll};
use huya::HuyaClient;
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
    let store = android_native_keyring_store::Store::new()
        .map_err(|error| format!("无法初始化 Android 安全存储：{error}"))?;

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
}

impl LiveClients {
    fn new() -> Result<Self, String> {
        Ok(Self {
            huya: HuyaClient::new()?,
            bilibili: BilibiliClient::new()?,
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
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SourcePlatform {
    Huya,
    Bilibili,
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
    Err("目前只支持虎牙和 Bilibili 直播间链接".to_string())
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
    initialize_secure_store().expect("failed to initialize the platform secure store");
    let live_clients = LiveClients::new().expect("failed to initialize live-source HTTP clients");
    let builder = tauri::Builder::default().plugin(tauri_plugin_clipboard_manager::init());

    #[cfg(target_os = "macos")]
    let builder = builder.enable_macos_default_menu(false);

    builder
        .manage(live_clients)
        .invoke_handler(tauri::generate_handler![
            resolve_live_stream,
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
    fn routes_supported_live_sources() {
        assert_eq!(
            source_platform("https://www.huya.com/196645").unwrap(),
            SourcePlatform::Huya
        );
        assert_eq!(
            source_platform("https://live.bilibili.com/5050").unwrap(),
            SourcePlatform::Bilibili
        );
        assert!(source_platform("https://example.com/5050").is_err());
    }
}
