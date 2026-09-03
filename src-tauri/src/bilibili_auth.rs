use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use base64::{engine::general_purpose::STANDARD, Engine as _};
use keyring_core::{Entry, Error as KeyringError};
use qrcode::{render::svg, QrCode};
use reqwest::{
    header::{COOKIE, SET_COOKIE},
    Client,
};
use serde::Serialize;
use serde_json::Value;
use url::Url;

const KEYRING_SERVICE: &str = "com.kiharari.simplelive.bilibili";
const KEYRING_ACCOUNT: &str = "web-cookie";
const QR_GENERATE_API: &str = "https://passport.bilibili.com/x/passport-login/web/qrcode/generate";
const QR_POLL_API: &str = "https://passport.bilibili.com/x/passport-login/web/qrcode/poll";
const ACCOUNT_API: &str = "https://api.bilibili.com/x/web-interface/nav";
const DEVICE_API: &str = "https://api.bilibili.com/x/frontend/finger/spi";
const QR_LIFETIME: Duration = Duration::from_secs(180);

pub struct BilibiliAuth {
    cookie_cache: Mutex<Option<Option<String>>>,
    device_cookie_cache: Mutex<Option<String>>,
    qr_session: Mutex<Option<QrSession>>,
}

struct QrSession {
    key: String,
    expires_at: Instant,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BilibiliAuthStatus {
    pub status: String,
    pub authenticated: bool,
    pub user_name: String,
    pub avatar_url: String,
    pub user_id: String,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BilibiliQrLogin {
    pub image_data_url: String,
    pub expires_in_seconds: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BilibiliQrPoll {
    pub status: String,
    pub message: String,
    pub auth: Option<BilibiliAuthStatus>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct BilibiliAccount {
    user_name: String,
    avatar_url: String,
    user_id: String,
}

impl BilibiliAuth {
    pub fn new() -> Self {
        Self {
            cookie_cache: Mutex::new(None),
            device_cookie_cache: Mutex::new(None),
            qr_session: Mutex::new(None),
        }
    }

    pub async fn request_cookie(&self, http: &Client) -> Result<String, String> {
        let user_cookie = self.cached_user_cookie()?;
        let device_cookie = self.device_cookie(http).await.unwrap_or_default();
        Ok(merge_cookie_strings(&[
            device_cookie.as_str(),
            user_cookie.as_deref().unwrap_or_default(),
        ]))
    }

    pub async fn status(&self, http: &Client) -> Result<BilibiliAuthStatus, String> {
        let Some(cookie) = self.load_user_cookie().await? else {
            return Ok(guest_status());
        };
        match validate_cookie(http, &cookie).await {
            Ok(Some(account)) => Ok(authenticated_status(account)),
            Ok(None) => Ok(BilibiliAuthStatus {
                status: "expired".to_string(),
                authenticated: false,
                user_name: String::new(),
                avatar_url: String::new(),
                user_id: String::new(),
                detail: "登录已失效，请重新扫码".to_string(),
            }),
            Err(error) => Ok(BilibiliAuthStatus {
                status: "unavailable".to_string(),
                authenticated: true,
                user_name: String::new(),
                avatar_url: String::new(),
                user_id: String::new(),
                detail: format!("已保存登录信息，但暂时无法验证：{error}"),
            }),
        }
    }

    pub async fn start_qr_login(&self, http: &Client) -> Result<BilibiliQrLogin, String> {
        let response = http
            .get(QR_GENERATE_API)
            .send()
            .await
            .map_err(|error| format!("无法生成 Bilibili 登录二维码：{error}"))?;
        if !response.status().is_success() {
            return Err(format!("Bilibili 登录接口返回状态 {}", response.status()));
        }
        let payload = response
            .json::<Value>()
            .await
            .map_err(|error| format!("Bilibili 登录接口返回无法识别的数据：{error}"))?;
        ensure_success(&payload, "生成二维码")?;
        let data = payload
            .get("data")
            .ok_or_else(|| "Bilibili 没有返回二维码数据".to_string())?;
        let qr_url = string_value(data.get("url"));
        let key = string_value(data.get("qrcode_key"));
        if qr_url.is_empty() || key.is_empty() {
            return Err("Bilibili 返回的二维码不完整".to_string());
        }
        let image_data_url = qr_svg_data_url(&qr_url)?;
        *self.qr_session.lock().map_err(lock_error)? = Some(QrSession {
            key,
            expires_at: Instant::now() + QR_LIFETIME,
        });
        Ok(BilibiliQrLogin {
            image_data_url,
            expires_in_seconds: QR_LIFETIME.as_secs(),
        })
    }

    pub async fn poll_qr_login(&self, http: &Client) -> Result<BilibiliQrPoll, String> {
        let session_snapshot = {
            let session = self.qr_session.lock().map_err(lock_error)?;
            let Some(session) = session.as_ref() else {
                return Ok(expired_poll());
            };
            (session.key.clone(), Instant::now() >= session.expires_at)
        };
        if session_snapshot.1 {
            *self.qr_session.lock().map_err(lock_error)? = None;
            return Ok(expired_poll());
        }
        let key = session_snapshot.0;

        let response = http
            .get(QR_POLL_API)
            .query(&[("qrcode_key", key.as_str())])
            .send()
            .await
            .map_err(|error| format!("无法查询 Bilibili 扫码状态：{error}"))?;
        if !response.status().is_success() {
            return Err(format!("Bilibili 登录接口返回状态 {}", response.status()));
        }
        let header_cookie = cookie_from_headers(response.headers());
        let payload = response
            .json::<Value>()
            .await
            .map_err(|error| format!("Bilibili 登录接口返回无法识别的数据：{error}"))?;
        ensure_success(&payload, "查询扫码状态")?;
        let data = payload.get("data").unwrap_or(&Value::Null);
        let code = value_as_i64(data.get("code")).unwrap_or(-1);
        match code {
            0 => {
                let redirect_cookie = cookie_from_redirect_url(&string_value(data.get("url")));
                let device_cookie = self.device_cookie(http).await.unwrap_or_default();
                let cookie = merge_cookie_strings(&[
                    device_cookie.as_str(),
                    redirect_cookie.as_str(),
                    header_cookie.as_str(),
                ]);
                if !cookie_has(&cookie, "SESSDATA") {
                    return Err("Bilibili 登录成功，但没有返回可用的 SESSDATA".to_string());
                }
                let account = validate_cookie(http, &cookie)
                    .await?
                    .ok_or_else(|| "Bilibili 登录信息未通过验证，请重新扫码".to_string())?;
                self.save_user_cookie(&cookie)?;
                *self.qr_session.lock().map_err(lock_error)? = None;
                Ok(BilibiliQrPoll {
                    status: "authenticated".to_string(),
                    message: "登录成功，正在重新获取最高画质".to_string(),
                    auth: Some(authenticated_status(account)),
                })
            }
            86090 => Ok(BilibiliQrPoll {
                status: "scanned".to_string(),
                message: "已扫码，请在手机上确认登录".to_string(),
                auth: None,
            }),
            86101 => Ok(BilibiliQrPoll {
                status: "unscanned".to_string(),
                message: "请使用哔哩哔哩客户端扫码".to_string(),
                auth: None,
            }),
            86038 => {
                *self.qr_session.lock().map_err(lock_error)? = None;
                Ok(expired_poll())
            }
            _ => Err(format!(
                "Bilibili 扫码登录失败：{}（状态 {code}）",
                string_value(data.get("message"))
            )),
        }
    }

    pub fn logout(&self) -> Result<BilibiliAuthStatus, String> {
        let entry = keyring_entry()?;
        match entry.delete_credential() {
            Ok(()) | Err(KeyringError::NoEntry) => {}
            Err(error) => return Err(format!("无法从系统钥匙串删除 Bilibili 登录信息：{error}")),
        }
        *self.cookie_cache.lock().map_err(lock_error)? = Some(None);
        *self.qr_session.lock().map_err(lock_error)? = None;
        Ok(guest_status())
    }

    fn cached_user_cookie(&self) -> Result<Option<String>, String> {
        Ok(self
            .cookie_cache
            .lock()
            .map_err(lock_error)?
            .as_ref()
            .cloned()
            .flatten())
    }

    async fn load_user_cookie(&self) -> Result<Option<String>, String> {
        {
            let cache = self.cookie_cache.lock().map_err(lock_error)?;
            if let Some(cookie) = cache.as_ref() {
                return Ok(cookie.clone());
            }
        }

        // A native secure store may take several seconds to authorize a read.
        // Keep that blocking OS call off the async runtime and,
        // crucially, do not hold the cache mutex while it is in progress. A
        // public stream can then start immediately and upgrade after auth loads.
        let cookie = tauri::async_runtime::spawn_blocking(read_keyring_cookie)
            .await
            .map_err(|error| format!("读取 Bilibili 登录信息的后台任务失败：{error}"))??;
        let mut cache = self.cookie_cache.lock().map_err(lock_error)?;
        if cache.is_none() {
            *cache = Some(cookie.clone());
        }
        Ok(cache.as_ref().cloned().flatten())
    }

    fn save_user_cookie(&self, cookie: &str) -> Result<(), String> {
        keyring_entry()?
            .set_password(cookie)
            .map_err(|error| format!("无法把 Bilibili 登录信息写入系统钥匙串：{error}"))?;
        *self.cookie_cache.lock().map_err(lock_error)? = Some(Some(cookie.to_string()));
        Ok(())
    }

    async fn device_cookie(&self, http: &Client) -> Result<String, String> {
        if let Some(cookie) = self.device_cookie_cache.lock().map_err(lock_error)?.clone() {
            return Ok(cookie);
        }
        let response = http
            .get(DEVICE_API)
            .send()
            .await
            .map_err(|error| format!("无法取得 Bilibili 设备标识：{error}"))?;
        if !response.status().is_success() {
            return Err(format!("Bilibili 设备接口返回状态 {}", response.status()));
        }
        let payload = response
            .json::<Value>()
            .await
            .map_err(|error| format!("Bilibili 设备接口返回无法识别的数据：{error}"))?;
        ensure_success(&payload, "取得设备标识")?;
        let data = payload.get("data").unwrap_or(&Value::Null);
        let buvid3 = format!("buvid3={}", string_value(data.get("b_3")));
        let buvid4 = format!("buvid4={}", string_value(data.get("b_4")));
        let cookie = merge_cookie_strings(&[buvid3.as_str(), buvid4.as_str()]);
        *self.device_cookie_cache.lock().map_err(lock_error)? = Some(cookie.clone());
        Ok(cookie)
    }
}

fn read_keyring_cookie() -> Result<Option<String>, String> {
    let entry = keyring_entry()?;
    Ok(match entry.get_password() {
        Ok(value) if !value.trim().is_empty() => Some(value),
        Ok(_) | Err(KeyringError::NoEntry) => None,
        Err(error) => return Err(format!("无法读取系统钥匙串中的 Bilibili 登录信息：{error}")),
    })
}

async fn validate_cookie(http: &Client, cookie: &str) -> Result<Option<BilibiliAccount>, String> {
    let response = http
        .get(ACCOUNT_API)
        .header(COOKIE, cookie)
        .send()
        .await
        .map_err(|error| format!("连接 Bilibili 帐号接口失败：{error}"))?;
    if !response.status().is_success() {
        return Err(format!("Bilibili 帐号接口返回状态 {}", response.status()));
    }
    let payload = response
        .json::<Value>()
        .await
        .map_err(|error| format!("Bilibili 帐号接口返回无法识别的数据：{error}"))?;
    if value_as_i64(payload.get("code")) != Some(0) {
        return Ok(None);
    }
    parse_account(payload.get("data").unwrap_or(&Value::Null))
}

fn parse_account(data: &Value) -> Result<Option<BilibiliAccount>, String> {
    if !data
        .get("isLogin")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return Ok(None);
    }
    let user_name = string_value(data.get("uname"));
    let user_id = string_value(data.get("mid"));
    if user_name.is_empty() || user_id.is_empty() {
        return Err("Bilibili 帐号信息不完整".to_string());
    }
    Ok(Some(BilibiliAccount {
        user_name,
        avatar_url: string_value(data.get("face")),
        user_id,
    }))
}

fn authenticated_status(account: BilibiliAccount) -> BilibiliAuthStatus {
    BilibiliAuthStatus {
        status: "authenticated".to_string(),
        authenticated: true,
        user_name: account.user_name,
        avatar_url: account.avatar_url,
        user_id: account.user_id,
        detail: "已登录，将按帐号权限自动请求最高画质".to_string(),
    }
}

fn guest_status() -> BilibiliAuthStatus {
    BilibiliAuthStatus {
        status: "guest".to_string(),
        authenticated: false,
        user_name: String::new(),
        avatar_url: String::new(),
        user_id: String::new(),
        detail: "未登录时平台通常只下发超清画质".to_string(),
    }
}

fn expired_poll() -> BilibiliQrPoll {
    BilibiliQrPoll {
        status: "expired".to_string(),
        message: "二维码已失效，请重新生成".to_string(),
        auth: None,
    }
}

fn keyring_entry() -> Result<Entry, String> {
    Entry::new(KEYRING_SERVICE, KEYRING_ACCOUNT)
        .map_err(|error| format!("无法访问系统钥匙串：{error}"))
}

fn qr_svg_data_url(value: &str) -> Result<String, String> {
    let code = QrCode::new(value.as_bytes())
        .map_err(|error| format!("无法编码 Bilibili 登录二维码：{error}"))?;
    let image = code
        .render::<svg::Color>()
        .min_dimensions(224, 224)
        .quiet_zone(true)
        .dark_color(svg::Color("#161412"))
        .light_color(svg::Color("#f2ede7"))
        .build();
    Ok(format!(
        "data:image/svg+xml;base64,{}",
        STANDARD.encode(image.as_bytes())
    ))
}

fn cookie_from_headers(headers: &reqwest::header::HeaderMap) -> String {
    let pairs = headers
        .get_all(SET_COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .filter_map(|value| value.split(';').next())
        .collect::<Vec<_>>();
    merge_cookie_strings(&pairs)
}

fn cookie_from_redirect_url(value: &str) -> String {
    let Ok(url) = Url::parse(value) else {
        return String::new();
    };
    let pairs = url
        .query_pairs()
        .filter(|(key, _)| is_login_cookie_name(key))
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<Vec<_>>();
    let refs = pairs.iter().map(String::as_str).collect::<Vec<_>>();
    merge_cookie_strings(&refs)
}

fn merge_cookie_strings(values: &[&str]) -> String {
    let mut pairs = BTreeMap::new();
    for value in values {
        for part in value.split(';') {
            let Some((name, content)) = part.trim().split_once('=') else {
                continue;
            };
            let name = name.trim();
            let content = content.trim();
            if name.is_empty() || content.is_empty() || !is_allowed_cookie_name(name) {
                continue;
            }
            pairs.insert(name.to_string(), content.to_string());
        }
    }
    pairs
        .into_iter()
        .map(|(name, value)| format!("{name}={value}"))
        .collect::<Vec<_>>()
        .join(";")
}

fn cookie_has(cookie: &str, name: &str) -> bool {
    cookie
        .split(';')
        .filter_map(|part| part.trim().split_once('='))
        .any(|(candidate, value)| candidate == name && !value.is_empty())
}

fn is_login_cookie_name(name: &str) -> bool {
    matches!(
        name,
        "SESSDATA" | "bili_jct" | "DedeUserID" | "DedeUserID__ckMd5" | "sid"
    )
}

fn is_allowed_cookie_name(name: &str) -> bool {
    is_login_cookie_name(name) || matches!(name, "buvid3" | "buvid4")
}

fn ensure_success(payload: &Value, operation: &str) -> Result<(), String> {
    let code = value_as_i64(payload.get("code")).unwrap_or(-1);
    if code == 0 {
        return Ok(());
    }
    Err(format!(
        "Bilibili {operation}失败：{}（状态 {code}）",
        string_value(payload.get("message"))
    ))
}

fn string_value(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(value)) => value.trim().to_string(),
        Some(Value::Number(value)) => value.to_string(),
        _ => String::new(),
    }
}

fn value_as_i64(value: Option<&Value>) -> Option<i64> {
    value.and_then(|value| {
        value
            .as_i64()
            .or_else(|| value.as_str().and_then(|value| value.parse().ok()))
    })
}

fn lock_error<T>(_: std::sync::PoisonError<T>) -> String {
    "Bilibili 登录状态暂时不可用".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::time::Instant;

    #[test]
    fn merges_only_required_login_and_device_cookies() {
        let cookie = merge_cookie_strings(&[
            "SESSDATA=secret; Path=/; tracking=drop",
            "buvid3=device-3; buvid4=device-4",
            "DedeUserID=42; SESSDATA=new-secret",
        ]);
        assert!(cookie.contains("SESSDATA=new-secret"));
        assert!(cookie.contains("DedeUserID=42"));
        assert!(cookie.contains("buvid3=device-3"));
        assert!(!cookie.contains("tracking"));
        assert!(!cookie.contains("Path"));
    }

    #[test]
    fn extracts_login_cookie_from_redirect_without_exposing_other_parameters() {
        let cookie = cookie_from_redirect_url(
            "https://passport.biligame.com/crossDomain?DedeUserID=42&SESSDATA=abc%2C123&gourl=https%3A%2F%2Fbilibili.com",
        );
        assert!(cookie.contains("DedeUserID=42"));
        assert!(cookie.contains("SESSDATA=abc,123"));
        assert!(!cookie.contains("gourl"));
    }

    #[test]
    fn parses_authenticated_account_without_serializing_cookie_material() {
        let account = parse_account(&json!({
            "isLogin": true,
            "uname": "viewer",
            "mid": 42,
            "face": "https://i0.hdslb.com/face.jpg"
        }))
        .unwrap()
        .unwrap();
        let status = authenticated_status(account);
        let serialized = serde_json::to_string(&status).unwrap();
        assert!(serialized.contains("viewer"));
        assert!(!serialized.contains("SESSDATA"));
        assert!(!serialized.contains("cookie"));
    }

    #[test]
    fn creates_an_embeddable_qr_svg_without_returning_the_raw_secret() {
        let result = qr_svg_data_url("https://passport.bilibili.com/example?key=secret").unwrap();
        assert!(result.starts_with("data:image/svg+xml;base64,"));
        assert!(!result.contains("secret"));
    }

    #[test]
    #[ignore = "requires the current public Bilibili QR login service"]
    fn generates_and_polls_a_live_login_qr() {
        tauri::async_runtime::block_on(async {
            let auth = BilibiliAuth::new();
            let http = Client::builder().build().unwrap();
            let login = auth.start_qr_login(&http).await.unwrap();
            assert!(login
                .image_data_url
                .starts_with("data:image/svg+xml;base64,"));
            let poll = auth.poll_qr_login(&http).await.unwrap();
            assert!(matches!(poll.status.as_str(), "unscanned" | "scanned"));
        });
    }

    #[test]
    #[ignore = "measures local keychain and current Bilibili device service"]
    fn measures_saved_login_and_device_cookie_startup() {
        tauri::async_runtime::block_on(async {
            let auth = BilibiliAuth::new();
            let http = Client::builder().build().unwrap();
            let started = Instant::now();
            let cookie = auth.load_user_cookie().await.unwrap();
            eprintln!(
                "keychain_cookie: {:?} (authenticated={})",
                started.elapsed(),
                cookie.as_deref().unwrap_or_default().contains("SESSDATA=")
            );
            let started = Instant::now();
            let device = auth.device_cookie(&http).await.unwrap();
            eprintln!(
                "device_cookie: {:?} (complete={})",
                started.elapsed(),
                device.contains("buvid3=") && device.contains("buvid4=")
            );
        });
    }
}
