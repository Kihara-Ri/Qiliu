//! Shared transport and result construction for Douyu and Douyin.
use crate::stream::{LiveStream, StreamLineOption, StreamQualityOption};
use reqwest::{Client, Response};
use serde_json::Value;
use std::time::Duration;
use url::Url;

pub const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0.0.0 Safari/537.36";

pub fn client(referer: &str) -> Result<Client, String> {
    Client::builder()
        .user_agent(USER_AGENT)
        .default_headers({
            let mut headers = reqwest::header::HeaderMap::new();
            headers.insert(
                reqwest::header::REFERER,
                referer.parse().map_err(|_| "无效的平台地址")?,
            );
            headers
        })
        .connect_timeout(Duration::from_secs(8))
        .timeout(Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| format!("无法初始化直播客户端：{e}"))
}

pub async fn body(response: Result<Response, reqwest::Error>) -> Result<String, String> {
    let mut response = response
        .map_err(|_| "直播平台连接失败，请稍后重试")?
        .error_for_status()
        .map_err(|_| "直播平台请求失败，请稍后重试")?;
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| "直播平台响应读取失败")? {
        if bytes.len() + chunk.len() > 8 * 1024 * 1024 {
            return Err("直播平台响应过大".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    String::from_utf8(bytes).map_err(|_| "直播平台响应编码无效".into())
}

pub async fn json(response: Result<Response, reqwest::Error>) -> Result<Value, String> {
    serde_json::from_str(&body(response).await?)
        .map_err(|_| "平台未返回有效数据，可能需要验证或稍后重试".into())
}

pub fn source_url(source: &str, hosts: &[&str]) -> Result<Url, String> {
    let normalized = if source.contains("://") {
        source.to_owned()
    } else {
        format!("https://{source}")
    };
    let url = Url::parse(normalized.trim()).map_err(|_| "直播间链接格式不正确")?;
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
        || !hosts.contains(&url.host_str().unwrap_or_default())
    {
        return Err("不支持的直播间链接".into());
    }
    Ok(url)
}

pub fn room_slug(url: &Url) -> Result<String, String> {
    let id = url
        .path_segments()
        .and_then(|mut p| p.find(|s| !s.is_empty()))
        .unwrap_or_default();
    if id.is_empty()
        || id.len() > 64
        || !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        return Err("链接中没有有效的直播间号".into());
    }
    Ok(id.into())
}

pub fn text(value: &Value) -> String {
    value
        .as_str()
        .map(str::to_owned)
        .or_else(|| value.as_u64().map(|n| n.to_string()))
        .unwrap_or_default()
}
pub fn number(value: &Value) -> Option<u64> {
    value.as_u64().or_else(|| value.as_str()?.parse().ok())
}

pub fn room(platform: &str, label: &str, id: String, source: String) -> LiveStream {
    LiveStream {
        platform: platform.into(),
        platform_label: label.into(),
        room_id: id,
        source_url: source,
        title: String::new(),
        anchor: String::new(),
        avatar_url: String::new(),
        is_live: false,
        is_replay: false,
        url: None,
        line_index: 0,
        line_count: 0,
        line_name: String::new(),
        line_options: vec![],
        quality_label: String::new(),
        quality_options: vec![],
        bitrate: 0,
        start_position_seconds: 0.0,
        format: String::new(),
    }
}

#[derive(Debug, Clone)]
pub struct Candidate {
    pub url: String,
    pub format: String,
    pub quality: u32,
    pub label: String,
}

pub fn select(
    stream: &mut LiveStream,
    candidates: &[Candidate],
    line: usize,
    quality: u32,
) -> Result<(), String> {
    let quality = if candidates.iter().any(|c| c.quality == quality) {
        quality
    } else {
        candidates
            .first()
            .map(|c| c.quality)
            .ok_or("直播间没有可用的 H.264 直播线路")?
    };
    let available: Vec<_> = candidates.iter().filter(|c| c.quality == quality).collect();
    if available.is_empty() {
        return Err("直播间没有可用的直播线路".into());
    }
    stream.line_index = line % available.len();
    stream.line_count = available.len();
    stream.line_options = available
        .iter()
        .enumerate()
        .map(|(index, c)| StreamLineOption {
            index,
            label: format!("线路 {} · {}", index + 1, c.format.to_uppercase()),
        })
        .collect();
    let chosen = available[stream.line_index];
    stream.url = Some(chosen.url.clone());
    stream.format = chosen.format.clone();
    stream.quality_label = chosen.label.clone();
    stream.line_name = stream.line_options[stream.line_index].label.clone();
    for c in candidates {
        if !stream.quality_options.iter().any(|q| q.value == c.quality) {
            stream.quality_options.push(StreamQualityOption {
                value: c.quality,
                label: c.label.clone(),
            });
        }
    }
    Ok(())
}

pub fn media_url(raw: &str, domains: &[&str]) -> Option<String> {
    let mut url = Url::parse(raw).ok()?;
    let host = url.host_str()?;
    if !matches!(url.scheme(), "https" | "http")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
        || !domains
            .iter()
            .any(|d| host == *d || host.ends_with(&format!(".{d}")))
    {
        return None;
    }
    url.set_scheme("https").ok()?;
    Some(url.into())
}

/// Parse one JSON value without destroying escaped titles, URLs or nested JSON strings.
pub fn json_after(input: &str, marker: &str) -> Option<Value> {
    let tail = input.split_once(marker)?.1.trim_start();
    serde_json::Deserializer::from_str(tail)
        .into_iter::<Value>()
        .next()?
        .ok()
}
