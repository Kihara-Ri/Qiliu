//! 版本更新通道：检测完整应用的新版本，桌面端确认后下载、验签、安装并重启，
//! Android 端引导到 GitHub 发布页。
//!
//! 安装包替换不触碰用户数据：收藏等保存在按 bundle identifier 键控的
//! localStorage 与系统安全存储中，与安装位置无关。

use serde::Serialize;
use std::sync::atomic::{AtomicU64, Ordering};
use tauri::{AppHandle, Emitter, WebviewWindow};
use url::Url;

// Android 端没有更新器插件，走 GitHub 发布 API 并引导到发布页。
#[cfg(not(desktop))]
const RELEASES_LATEST_API: &str = "https://api.github.com/repos/Kihara-Ri/Qiliu/releases/latest";
const RELEASES_PAGE: &str = "https://github.com/Kihara-Ri/Qiliu/releases/latest";

#[derive(Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AppUpdateInfo {
    /// "desktop" 走应用内更新器；"android" 引导到发布页。
    pub channel: String,
    pub current_version: String,
    pub available: bool,
    pub version: Option<String>,
    pub notes: Option<String>,
    pub pub_date: Option<String>,
    pub can_auto_install: bool,
    pub release_url: Option<String>,
}

impl AppUpdateInfo {
    fn up_to_date(channel: &str, current_version: &str, release_url: &str) -> Self {
        Self {
            channel: channel.to_string(),
            current_version: current_version.to_string(),
            available: false,
            version: None,
            notes: None,
            pub_date: None,
            can_auto_install: false,
            release_url: Some(release_url.to_string()),
        }
    }
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
struct DownloadProgress {
    downloaded: u64,
    total: Option<u64>,
    finished: bool,
}

#[tauri::command]
pub async fn check_app_update(app: AppHandle) -> Result<AppUpdateInfo, String> {
    let current = app.package_info().version.to_string();

    #[cfg(desktop)]
    return check_with_updater(&app, &current).await;

    #[cfg(not(desktop))]
    return check_github_latest(&current).await;
}

/// 下载并安装桌面端更新；进度通过 `app-update://progress` 事件推送给前端，
/// 安装完成后应用自动重启。非桌面平台由前端按 canAutoInstall 分流，不调用本命令。
#[tauri::command]
pub async fn install_app_update(app: AppHandle, window: WebviewWindow) -> Result<(), String> {
    #[cfg(desktop)]
    return install_app_update_desktop(app, window).await;

    #[cfg(not(desktop))]
    {
        let _ = (app, window);
        Err("此平台不支持应用内安装更新，请前往发布页下载".to_string())
    }
}

/// 在系统浏览器打开发布页；只接受 GitHub 域，避免把任意 URL 交给 opener。
#[tauri::command]
pub async fn open_release_page(app: AppHandle, url: String) -> Result<(), String> {
    let parsed = Url::parse(&url).map_err(|_| "链接格式不正确".to_string())?;
    let host = parsed.host_str().unwrap_or_default();
    if parsed.scheme() != "https" || !(host == "github.com" || host.ends_with(".github.com")) {
        return Err("只允许打开 GitHub 发布页".to_string());
    }
    use tauri_plugin_opener::OpenerExt;
    app.opener()
        .open_url(parsed.as_str(), None::<&str>)
        .map_err(|error| format!("打开发布页失败：{error}"))
}

#[cfg(desktop)]
async fn check_with_updater(app: &AppHandle, current: &str) -> Result<AppUpdateInfo, String> {
    use tauri_plugin_updater::UpdaterExt;

    let updater = app
        .updater_builder()
        .build()
        .map_err(|error| format!("更新器初始化失败：{error}"))?;
    let update = updater
        .check()
        .await
        .map_err(|error| format!("检查更新失败：{error}"))?;
    let Some(update) = update else {
        return Ok(AppUpdateInfo::up_to_date("desktop", current, RELEASES_PAGE));
    };
    Ok(AppUpdateInfo {
        channel: "desktop".to_string(),
        current_version: current.to_string(),
        available: true,
        version: Some(update.version.clone()),
        notes: update.body.clone(),
        pub_date: update.date.map(|date| date.unix_timestamp().to_string()),
        can_auto_install: true,
        release_url: Some(RELEASES_PAGE.to_string()),
    })
}

#[cfg(desktop)]
async fn install_app_update_desktop(
    app: AppHandle,
    window: WebviewWindow,
) -> Result<(), String> {
    use tauri_plugin_updater::UpdaterExt;

    let updater = app
        .updater_builder()
        .build()
        .map_err(|error| format!("更新器初始化失败：{error}"))?;
    let update = updater
        .check()
        .await
        .map_err(|error| format!("检查更新失败：{error}"))?
        .ok_or("当前没有可用的应用更新")?;

    let downloaded = AtomicU64::new(0);
    let bytes = update
        .download(
            |chunk, total| {
                let downloaded = downloaded.fetch_add(chunk as u64, Ordering::Relaxed)
                    + chunk as u64;
                let _ = window.emit(
                    "app-update://progress",
                    DownloadProgress {
                        downloaded,
                        total,
                        finished: false,
                    },
                );
            },
            || {
                let _ = window.emit(
                    "app-update://progress",
                    DownloadProgress {
                        downloaded: downloaded.load(Ordering::Relaxed),
                        total: None,
                        finished: true,
                    },
                );
            },
        )
        .await
        .map_err(|error| format!("下载更新失败：{error}"))?;
    // Windows 上 install 启动安装器后会退出应用；macOS/Linux 安装完由 restart 拉起新版本。
    update
        .install(bytes)
        .map_err(|error| format!("安装更新失败：{error}"))?;
    app.restart()
}

#[cfg(not(desktop))]
async fn check_github_latest(current: &str) -> Result<AppUpdateInfo, String> {
    use std::time::Duration;

    let client = reqwest::Client::builder()
        .user_agent(concat!("QiliuUpdater/", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|error| format!("初始化更新检查失败：{error}"))?;
    let release: serde_json::Value = client
        .get(RELEASES_LATEST_API)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|error| format!("检查更新失败：{error}"))?
        .error_for_status()
        .map_err(|error| format!("检查更新失败：{error}"))?
        .json()
        .await
        .map_err(|error| format!("读取发布信息失败：{error}"))?;
    let release_url = release["html_url"]
        .as_str()
        .unwrap_or(RELEASES_PAGE)
        .to_string();
    let Some(tag) = release["tag_name"].as_str() else {
        return Err("发布信息缺少版本号".to_string());
    };
    let version = tag.trim_start_matches('v').to_string();
    if !crate::patch_hub::version_newer(&version, current) {
        return Ok(AppUpdateInfo::up_to_date("android", current, &release_url));
    }
    Ok(AppUpdateInfo {
        channel: "android".to_string(),
        current_version: current.to_string(),
        available: true,
        version: Some(version),
        notes: release["body"].as_str().map(str::to_string),
        pub_date: release["published_at"].as_str().map(str::to_string),
        can_auto_install: false,
        release_url: Some(release_url),
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn latest_release_version_comparison() {
        assert!(crate::patch_hub::version_newer("1.6.0", "1.5.0"));
        assert!(!crate::patch_hub::version_newer("1.5.0", "1.5.0"));
        assert!(!crate::patch_hub::version_newer("garbage", "1.5.0"));
    }
}
