use std::collections::HashSet;
use std::time::Duration;

use reqwest::{
    header::{HeaderMap, HeaderValue, ACCEPT, COOKIE, ORIGIN, REFERER, USER_AGENT},
    Client,
};
use serde_json::Value;
use url::Url;

use crate::bilibili_auth::{BilibiliAuth, BilibiliAuthStatus, BilibiliQrLogin, BilibiliQrPoll};
use crate::stream::{LiveStream, StreamLineOption, StreamQualityOption};

const BILIBILI_ROOM_INFO_API: &str = "https://api.live.bilibili.com/room/v1/Room/get_info";
const BILIBILI_MASTER_INFO_API: &str = "https://api.live.bilibili.com/live_user/v1/Master/info";
const BILIBILI_PLAY_INFO_API: &str =
    "https://api.live.bilibili.com/xlive/web-room/v2/index/getRoomPlayInfo";
const BILIBILI_USER_AGENT: &str =
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36";

pub struct BilibiliClient {
    http: Client,
    auth: BilibiliAuth,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PlayCandidate {
    url: String,
    protocol: String,
    format: String,
    codec: String,
    line_name: String,
    current_qn: u32,
    is_mcdn: bool,
}

impl BilibiliClient {
    pub fn new() -> Result<Self, String> {
        let mut headers = HeaderMap::new();
        headers.insert(
            ACCEPT,
            HeaderValue::from_static("application/json, text/plain, */*"),
        );
        headers.insert(
            ORIGIN,
            HeaderValue::from_static("https://live.bilibili.com"),
        );
        headers.insert(
            REFERER,
            HeaderValue::from_static("https://live.bilibili.com/"),
        );
        headers.insert(USER_AGENT, HeaderValue::from_static(BILIBILI_USER_AGENT));

        let http = Client::builder()
            .default_headers(headers)
            .connect_timeout(Duration::from_secs(8))
            .timeout(Duration::from_secs(15))
            .tcp_keepalive(Duration::from_secs(30))
            .pool_idle_timeout(Duration::from_secs(90))
            .pool_max_idle_per_host(2)
            .redirect(reqwest::redirect::Policy::limited(3))
            .build()
            .map_err(|error| format!("无法初始化 Bilibili 网络客户端：{error}"))?;

        Ok(Self {
            http,
            auth: BilibiliAuth::new(),
        })
    }

    pub async fn resolve(
        &self,
        source: &str,
        requested_line_index: usize,
        bitrate: u32,
    ) -> Result<LiveStream, String> {
        let input_room_id = parse_room_id(source)?;
        let room_info = self
            .get_public_json(
                BILIBILI_ROOM_INFO_API,
                &[("room_id", input_room_id.as_str())],
            )
            .await?;
        ensure_bilibili_success(&room_info, "房间资料")?;
        let room_data = room_info.get("data").unwrap_or(&Value::Null);
        let room_id = value_as_u64(room_data.get("room_id"))
            .filter(|value| *value > 0)
            .map(|value| value.to_string())
            .unwrap_or(input_room_id);
        let title = string_value(room_data.get("title"));
        let avatar_fallback = non_empty_or(
            string_value(room_data.get("user_cover")),
            string_value(room_data.get("keyframe")),
        );
        let live_status = value_as_i64(room_data.get("live_status")).unwrap_or_default();
        let uid = value_as_u64(room_data.get("uid"))
            .filter(|value| *value > 0)
            .unwrap_or_default();

        let (anchor, avatar_url) = self.resolve_anchor(uid, avatar_fallback).await;
        let source_url = format!("https://live.bilibili.com/{room_id}");
        let is_live = live_status == 1;
        let is_replay = live_status == 2;

        if !is_live && !is_replay {
            return Ok(LiveStream {
                platform: "bilibili".to_string(),
                platform_label: "Bilibili".to_string(),
                source_url,
                room_id,
                title,
                anchor,
                avatar_url,
                is_live: false,
                is_replay: false,
                url: None,
                line_index: 0,
                line_count: 0,
                line_name: String::new(),
                line_options: Vec::new(),
                quality_label: String::new(),
                quality_options: Vec::new(),
                bitrate: 0,
                start_position_seconds: 0.0,
                format: String::new(),
            });
        }

        let (play_info, requested_qn) = if bitrate == 0 {
            let discovery = self.request_play_info(&room_id, 0).await?;
            let discovery_playurl = playurl_payload(&discovery)?;
            let highest_qn = highest_available_avc_qn(discovery_playurl).unwrap_or(10_000);
            (
                self.request_play_info(&room_id, highest_qn).await?,
                highest_qn,
            )
        } else {
            (
                self.request_play_info(&room_id, bitrate).await?,
                bitrate,
            )
        };
        ensure_bilibili_success(&play_info, "播放线路")?;
        let playurl = playurl_payload(&play_info)?;
        let candidates = extract_play_candidates(playurl, requested_qn)?;
        if candidates.is_empty() {
            return Err("当前 Bilibili 直播间没有可用的 H.264 FLV 线路".to_string());
        }

        let line_index = requested_line_index % candidates.len();
        let candidate = &candidates[line_index];
        let quality_label = quality_label(playurl, candidate.current_qn);
        let quality_options = available_qualities(playurl, candidate.current_qn);
        let line_options = available_lines(&candidates);
        let transport = if candidate.protocol.eq_ignore_ascii_case("http_hls") {
            "HLS"
        } else {
            "FLV"
        };

        Ok(LiveStream {
            platform: "bilibili".to_string(),
            platform_label: "Bilibili".to_string(),
            source_url,
            room_id,
            title,
            anchor,
            avatar_url,
            is_live,
            is_replay,
            url: Some(candidate.url.clone()),
            line_index,
            line_count: candidates.len(),
            line_name: format!("{} · {transport}", candidate.line_name),
            line_options,
            quality_label,
            quality_options,
            bitrate: 0,
            start_position_seconds: 0.0,
            format: candidate.format.clone(),
        })
    }

    async fn get_json(&self, endpoint: &str, query: &[(&str, &str)]) -> Result<Value, String> {
        let cookie = self
            .auth
            .request_cookie(&self.http)
            .await
            .unwrap_or_default();
        self.send_json(endpoint, query, cookie).await
    }

    async fn get_public_json(
        &self,
        endpoint: &str,
        query: &[(&str, &str)],
    ) -> Result<Value, String> {
        self.send_json(endpoint, query, String::new()).await
    }

    async fn send_json(
        &self,
        endpoint: &str,
        query: &[(&str, &str)],
        cookie: String,
    ) -> Result<Value, String> {
        let mut request = self.http.get(endpoint).query(query);
        if !cookie.is_empty() {
            request = request.header(COOKIE, cookie);
        }
        let response = request
            .send()
            .await
            .map_err(|error| format!("连接 Bilibili 失败：{error}"))?;
        if !response.status().is_success() {
            return Err(format!("Bilibili 接口返回状态 {}", response.status()));
        }
        response
            .json::<Value>()
            .await
            .map_err(|error| format!("Bilibili 返回了无法识别的数据：{error}"))
    }

    async fn request_play_info(&self, room_id: &str, qn: u32) -> Result<Value, String> {
        let qn = qn.to_string();
        self.get_json(
            BILIBILI_PLAY_INFO_API,
            &[
                ("room_id", room_id),
                ("protocol", "0,1"),
                ("format", "0,1,2"),
                ("codec", "0"),
                ("qn", qn.as_str()),
                ("platform", "web"),
                ("ptype", "8"),
                ("dolby", "5"),
                ("panorama", "1"),
                ("mask", "0"),
                ("no_playurl", "0"),
            ],
        )
        .await
    }

    pub async fn auth_status(&self) -> Result<BilibiliAuthStatus, String> {
        self.auth.status(&self.http).await
    }

    pub async fn start_qr_login(&self) -> Result<BilibiliQrLogin, String> {
        self.auth.start_qr_login(&self.http).await
    }

    pub async fn poll_qr_login(&self) -> Result<BilibiliQrPoll, String> {
        self.auth.poll_qr_login(&self.http).await
    }

    pub fn logout(&self) -> Result<BilibiliAuthStatus, String> {
        self.auth.logout()
    }

    async fn resolve_anchor(&self, uid: u64, avatar_fallback: String) -> (String, String) {
        if uid == 0 {
            return (String::new(), avatar_fallback);
        }
        let uid = uid.to_string();
        let Ok(payload) = self
            .get_public_json(BILIBILI_MASTER_INFO_API, &[("uid", uid.as_str())])
            .await
        else {
            return (String::new(), avatar_fallback);
        };
        if ensure_bilibili_success(&payload, "主播资料").is_err() {
            return (String::new(), avatar_fallback);
        }
        let data = payload.get("data").unwrap_or(&Value::Null);
        (
            string_value(data.get("info").and_then(|value| value.get("uname"))),
            non_empty_or(
                string_value(data.get("info").and_then(|value| value.get("face"))),
                avatar_fallback,
            ),
        )
    }
}

fn parse_room_id(source: &str) -> Result<String, String> {
    let source = source.trim();
    if source.is_empty() {
        return Err("请输入 Bilibili 直播间链接".to_string());
    }
    let normalized = if source.contains("://") {
        source.to_string()
    } else {
        format!("https://{source}")
    };
    let url = Url::parse(&normalized).map_err(|_| "直播间链接格式不正确".to_string())?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err("只支持 http 或 https 的 Bilibili 链接".to_string());
    }
    let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
    if host == "live.bilibili.com" || host.ends_with(".live.bilibili.com") {
        let room_id = url
            .path_segments()
            .and_then(|mut segments| segments.find(|segment| !segment.is_empty()))
            .unwrap_or_default();
        if is_numeric_room_id(room_id) {
            return Ok(room_id.to_string());
        }
    }
    if host == "bilibili.com" || host.ends_with(".bilibili.com") {
        if let Some(room_id) = url
            .query_pairs()
            .find_map(|(key, value)| (key == "cid" && is_numeric_room_id(&value)).then(|| value))
        {
            return Ok(room_id.into_owned());
        }
    }
    Err("链接中没有有效的 Bilibili 直播间号".to_string())
}

fn is_numeric_room_id(value: &str) -> bool {
    !value.is_empty() && value.len() <= 20 && value.bytes().all(|byte| byte.is_ascii_digit())
}

fn ensure_bilibili_success(payload: &Value, operation: &str) -> Result<(), String> {
    let code = value_as_i64(payload.get("code")).unwrap_or(-1);
    if code == 0 {
        return Ok(());
    }
    let message = string_value(payload.get("message"));
    if message.is_empty() {
        Err(format!("Bilibili {operation}失败（状态 {code}）"))
    } else {
        Err(format!(
            "Bilibili {operation}失败：{message}（状态 {code}）"
        ))
    }
}

fn playurl_payload(payload: &Value) -> Result<&Value, String> {
    ensure_bilibili_success(payload, "播放线路")?;
    payload
        .pointer("/data/playurl_info/playurl")
        .ok_or_else(|| "Bilibili 没有返回播放地址".to_string())
}

fn highest_available_avc_qn(playurl: &Value) -> Option<u32> {
    playurl
        .get("stream")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .flat_map(|stream| {
            stream
                .get("format")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
        })
        .flat_map(|format| {
            format
                .get("codec")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
        })
        .filter(|codec| string_value(codec.get("codec_name")).eq_ignore_ascii_case("avc"))
        .flat_map(|codec| {
            codec
                .get("accept_qn")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
        })
        .filter_map(|value| value_as_u64(Some(value)))
        .filter_map(|value| u32::try_from(value).ok())
        .max()
}

fn extract_play_candidates(
    playurl: &Value,
    requested_qn: u32,
) -> Result<Vec<PlayCandidate>, String> {
    let mut candidates = Vec::new();
    let mut seen = HashSet::new();
    for stream in playurl
        .get("stream")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let protocol = string_value(stream.get("protocol_name"));
        for format in stream
            .get("format")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let format_name = string_value(format.get("format_name"));
            for codec in format
                .get("codec")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let codec_name = string_value(codec.get("codec_name"));
                if !codec_name.eq_ignore_ascii_case("avc") {
                    continue;
                }
                let base_url = string_value(codec.get("base_url"));
                let current_qn = value_as_u64(codec.get("current_qn"))
                    .and_then(|value| u32::try_from(value).ok())
                    .unwrap_or_default();
                for url_info in codec
                    .get("url_info")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    let host = string_value(url_info.get("host"));
                    let extra = string_value(url_info.get("extra"));
                    let raw_url = format!("{host}{base_url}{extra}");
                    let url = validated_media_url(&raw_url)?;
                    let canonical_url = url.to_string();
                    if !seen.insert(canonical_url.clone()) {
                        continue;
                    }
                    let line_name = url
                        .query_pairs()
                        .find_map(|(key, value)| (key == "cdn").then(|| value.into_owned()))
                        .filter(|value| !value.is_empty())
                        .unwrap_or_else(|| compact_host_name(url.host_str().unwrap_or_default()));
                    let is_mcdn = host.to_ascii_lowercase().contains("mcdn")
                        || line_name.to_ascii_lowercase().contains("mcdn");
                    candidates.push(PlayCandidate {
                        url: canonical_url,
                        protocol: protocol.clone(),
                        format: format_name.clone(),
                        codec: codec_name.clone(),
                        line_name,
                        current_qn,
                        is_mcdn,
                    });
                }
            }
        }
    }
    let has_live_flv = candidates.iter().any(|candidate| {
        candidate.protocol.eq_ignore_ascii_case("http_stream")
            && candidate.format.eq_ignore_ascii_case("flv")
    });
    if has_live_flv {
        candidates.retain(|candidate| {
            candidate.protocol.eq_ignore_ascii_case("http_stream")
                && candidate.format.eq_ignore_ascii_case("flv")
        });
    }
    candidates.sort_by_key(|candidate| candidate_rank(candidate, requested_qn));
    let Some(applied_qn) = candidates.first().map(|candidate| candidate.current_qn) else {
        return Ok(candidates);
    };
    candidates.retain(|candidate| candidate.current_qn == applied_qn);
    Ok(candidates)
}

fn candidate_rank(candidate: &PlayCandidate, requested_qn: u32) -> (u8, u8, u8, u8, u8) {
    (
        u8::from(candidate.current_qn != requested_qn),
        u8::from(candidate.is_mcdn),
        u8::from(!candidate.protocol.eq_ignore_ascii_case("http_stream")),
        u8::from(!candidate.format.eq_ignore_ascii_case("flv")),
        u8::from(!candidate.codec.eq_ignore_ascii_case("avc")),
    )
}

fn validated_media_url(value: &str) -> Result<Url, String> {
    let url = Url::parse(value).map_err(|_| "Bilibili 媒体地址格式不正确".to_string())?;
    if url.scheme() != "https" {
        return Err("Bilibili 媒体地址只允许 HTTPS".to_string());
    }
    let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
    let allowed = host == "bilivideo.com"
        || host.ends_with(".bilivideo.com")
        || host == "bilibili.com"
        || host.ends_with(".bilibili.com");
    if !allowed {
        return Err("Bilibili 返回了不受信任的媒体域名".to_string());
    }
    Ok(url)
}

fn compact_host_name(host: &str) -> String {
    let first = host.split('.').next().unwrap_or_default();
    first
        .split("--")
        .next()
        .filter(|value| !value.is_empty())
        .unwrap_or("默认")
        .to_ascii_uppercase()
}

fn quality_label(playurl: &Value, current_qn: u32) -> String {
    let description = playurl
        .get("g_qn_desc")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .find(|entry| value_as_u64(entry.get("qn")) == Some(u64::from(current_qn)))
        .map(|entry| string_value(entry.get("desc")))
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| match current_qn {
            10_000 => "原画".to_string(),
            400 => "蓝光".to_string(),
            250 => "超清".to_string(),
            150 => "高清".to_string(),
            80 => "流畅".to_string(),
            _ => "实际画质".to_string(),
        });
    format!("{description} · QN {current_qn}")
}

fn available_qualities(playurl: &Value, current_qn: u32) -> Vec<StreamQualityOption> {
    let mut qualities = playurl
        .get("g_qn_desc")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            let value = value_as_u64(entry.get("qn")).and_then(|value| u32::try_from(value).ok())?;
            if value == 0 {
                return None;
            }
            let description = string_value(entry.get("desc"));
            let label = if description.is_empty() {
                format!("QN {value}")
            } else {
                format!("{description} · QN {value}")
            };
            Some(StreamQualityOption { value, label })
        })
        .collect::<Vec<_>>();

    let accepted_qns = playurl
        .get("stream")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .flat_map(|stream| {
            stream
                .get("format")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
        })
        .flat_map(|format| {
            format
                .get("codec")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
        })
        .filter(|codec| string_value(codec.get("codec_name")).eq_ignore_ascii_case("avc"))
        .flat_map(|codec| {
            codec
                .get("accept_qn")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
        })
        .filter_map(|value| value_as_u64(Some(value)))
        .filter_map(|value| u32::try_from(value).ok())
        .filter(|value| *value > 0)
        .collect::<HashSet<_>>();
    for value in accepted_qns {
        if qualities.iter().any(|option| option.value == value) {
            continue;
        }
        qualities.push(StreamQualityOption {
            value,
            label: quality_label(playurl, value),
        });
    }

    if !qualities.iter().any(|option| option.value == current_qn) && current_qn > 0 {
        qualities.push(StreamQualityOption {
            value: current_qn,
            label: quality_label(playurl, current_qn),
        });
    }
    qualities.sort_by(|left, right| right.value.cmp(&left.value));
    qualities.dedup_by_key(|option| option.value);
    qualities.insert(
        0,
        StreamQualityOption {
            value: 0,
            label: "自动 · 最高可用".to_string(),
        },
    );
    qualities
}

fn available_lines(candidates: &[PlayCandidate]) -> Vec<StreamLineOption> {
    candidates
        .iter()
        .enumerate()
        .map(|(index, candidate)| {
            let transport = if candidate.protocol.eq_ignore_ascii_case("http_hls") {
                "HLS"
            } else {
                "FLV"
            };
            StreamLineOption {
                index,
                label: format!("{} · {transport} · 线路 {}", candidate.line_name, index + 1),
            }
        })
        .collect()
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

fn value_as_u64(value: Option<&Value>) -> Option<u64> {
    value.and_then(|value| {
        value
            .as_u64()
            .or_else(|| value.as_str().and_then(|value| value.parse().ok()))
    })
}

fn non_empty_or(primary: String, fallback: String) -> String {
    if primary.is_empty() {
        fallback
    } else {
        primary
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::time::Instant;

    #[test]
    fn parses_supported_bilibili_room_links() {
        assert_eq!(
            parse_room_id("https://live.bilibili.com/5050?live_from=81001").unwrap(),
            "5050"
        );
        assert_eq!(parse_room_id("live.bilibili.com/6").unwrap(), "6");
        assert_eq!(
            parse_room_id(
                "https://www.bilibili.com/blackboard/live/live-activity-player.html?cid=5050"
            )
            .unwrap(),
            "5050"
        );
    }

    #[test]
    fn rejects_non_bilibili_or_non_numeric_rooms() {
        assert!(parse_room_id("https://www.huya.com/5050").is_err());
        assert!(parse_room_id("https://live.bilibili.com/not-a-room").is_err());
        assert!(parse_room_id("https://live.bilibili.com/").is_err());
    }

    #[test]
    fn selects_avc_flv_and_prefers_non_mcdn_without_faking_requested_quality() {
        let playurl = json!({
            "g_qn_desc": [
                {"qn": 10000, "desc": "原画"},
                {"qn": 250, "desc": "超清"}
            ],
            "stream": [
                {
                    "protocol_name": "http_hls",
                    "format": [{"format_name": "fmp4", "codec": [{
                        "codec_name": "avc", "current_qn": 250, "accept_qn": [10000, 400, 250], "base_url": "/live/test.m3u8",
                        "url_info": [{"host": "https://hls.bilivideo.com", "extra": "?cdn=hls"}]
                    }]}]
                },
                {
                    "protocol_name": "http_stream",
                    "format": [{"format_name": "flv", "codec": [{
                        "codec_name": "avc", "current_qn": 250, "accept_qn": [10000, 400, 250], "base_url": "/live/test.flv",
                        "url_info": [
                            {"host": "https://mcdn.bilivideo.com", "extra": "?cdn=mcdn"},
                            {"host": "https://good.bilivideo.com", "extra": "?cdn=good"}
                        ]
                    }]}]
                }
            ]
        });

        assert_eq!(highest_available_avc_qn(&playurl), Some(10_000));
        let candidates = extract_play_candidates(&playurl, 10_000).unwrap();
        assert!(candidates.iter().all(|candidate| candidate.format == "flv"));
        assert_eq!(candidates[0].format, "flv");
        assert_eq!(candidates[0].line_name, "good");
        assert_eq!(candidates[0].current_qn, 250);
        assert_eq!(
            quality_label(&playurl, candidates[0].current_qn),
            "超清 · QN 250"
        );
    }

    #[test]
    fn keeps_only_the_server_applied_quality_after_ranking_the_requested_tier() {
        let playurl = json!({
            "stream": [{
                "protocol_name": "http_stream",
                "format": [{"format_name": "flv", "codec": [
                    {
                        "codec_name": "avc", "current_qn": 400, "accept_qn": [10000, 400],
                        "base_url": "/live/400.flv",
                        "url_info": [{"host": "https://blue.bilivideo.com", "extra": "?cdn=blue"}]
                    },
                    {
                        "codec_name": "avc", "current_qn": 10000, "accept_qn": [10000, 400],
                        "base_url": "/live/source.flv",
                        "url_info": [{"host": "https://source.bilivideo.com", "extra": "?cdn=source"}]
                    }
                ]}]
            }]
        });
        let candidates = extract_play_candidates(&playurl, 10_000).unwrap();
        assert!(!candidates.is_empty());
        assert!(candidates
            .iter()
            .all(|candidate| candidate.current_qn == 10_000));
    }

    #[test]
    fn exposes_manual_quality_and_line_choices_from_the_applied_response() {
        let playurl = json!({
            "g_qn_desc": [
                {"qn": 10000, "desc": "原画"},
                {"qn": 250, "desc": "超清"}
            ],
            "stream": [{
                "protocol_name": "http_stream",
                "format": [{"format_name": "flv", "codec": [{
                    "codec_name": "avc",
                    "current_qn": 10000,
                    "accept_qn": [10000, 400, 250],
                    "base_url": "/live/source.flv",
                    "url_info": [
                        {"host": "https://first.bilivideo.com", "extra": "?cdn=first"},
                        {"host": "https://second.bilivideo.com", "extra": "?cdn=second"}
                    ]
                }]}]
            }]
        });

        let qualities = available_qualities(&playurl, 10_000);
        assert_eq!(qualities[0].value, 0);
        assert_eq!(qualities[0].label, "自动 · 最高可用");
        assert_eq!(
            qualities.iter().map(|option| option.value).collect::<Vec<_>>(),
            [0, 10_000, 400, 250]
        );

        let candidates = extract_play_candidates(&playurl, 10_000).unwrap();
        let lines = available_lines(&candidates);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].index, 0);
        assert_eq!(lines[0].label, "first · FLV · 线路 1");
        assert_eq!(lines[1].label, "second · FLV · 线路 2");
    }

    #[test]
    fn rejects_untrusted_media_hosts() {
        assert!(validated_media_url("https://example.com/live.flv").is_err());
        assert!(validated_media_url("http://cn.bilivideo.com/live.flv").is_err());
        assert!(validated_media_url("https://cn.bilivideo.com/live.flv").is_ok());
    }

    #[test]
    #[ignore = "requires a currently live public Bilibili room and network access"]
    fn resolves_and_opens_a_current_bilibili_live_flv() {
        tauri::async_runtime::block_on(async {
            let client = BilibiliClient::new().unwrap();
            let room = std::env::var("SIMPLE_LIVE_BILIBILI_TEST_ROOM")
                .unwrap_or_else(|_| "5050".to_string());
            let stream = client
                .resolve(&format!("https://live.bilibili.com/{room}"), 0, 0)
                .await
                .unwrap();
            assert!(stream.is_live, "the integration-test room is not live");
            assert_eq!(stream.platform, "bilibili");
            assert_eq!(stream.format, "flv");
            assert!(stream.quality_label.contains("QN"));
            eprintln!("resolved Bilibili test room at {}", stream.quality_label);

            let response = client
                .http
                .get(stream.url.as_deref().unwrap())
                .send()
                .await
                .unwrap();
            assert!(response.status().is_success());
            assert_eq!(
                response
                    .headers()
                    .get(reqwest::header::CONTENT_TYPE)
                    .and_then(|value| value.to_str().ok()),
                Some("video/x-flv")
            );
        });
    }

    #[test]
    #[ignore = "requires a currently live public Bilibili room and authenticated quality state"]
    fn resolves_current_bilibili_stable_quality() {
        tauri::async_runtime::block_on(async {
            let client = BilibiliClient::new().unwrap();
            let room = std::env::var("SIMPLE_LIVE_BILIBILI_TEST_ROOM")
                .unwrap_or_else(|_| "5050".to_string());
            let _ = client.auth.status(&client.http).await.unwrap();
            let stream = client
                .resolve(&format!("https://live.bilibili.com/{room}"), 0, 400)
                .await
                .unwrap();
            assert!(stream.is_live, "the integration-test room is not live");
            assert_eq!(stream.format, "flv");
            assert!(
                stream.quality_label.contains("QN 400")
                    || stream.quality_label.contains("QN 250")
                    || stream.quality_label.contains("QN 150")
                    || stream.quality_label.contains("QN 80"),
                "stable request unexpectedly resolved {}",
                stream.quality_label
            );
            eprintln!(
                "stable Bilibili quality resolved at {}",
                stream.quality_label
            );
        });
    }

    #[test]
    #[ignore = "requires a currently live public Bilibili room and network access"]
    fn measures_bilibili_startup_waterfall() {
        tauri::async_runtime::block_on(async {
            let client = BilibiliClient::new().unwrap();
            let room = std::env::var("SIMPLE_LIVE_BILIBILI_TEST_ROOM")
                .unwrap_or_else(|_| "22907643".to_string());
            let started = Instant::now();
            let mut checkpoint = started;

            let cookie = client.auth.request_cookie(&client.http).await.unwrap();
            eprintln!(
                "auth_and_device_cookie: {:?} (authenticated={})",
                checkpoint.elapsed(),
                cookie.contains("SESSDATA=")
            );
            checkpoint = Instant::now();

            let room_info = client
                .get_public_json(BILIBILI_ROOM_INFO_API, &[("room_id", room.as_str())])
                .await
                .unwrap();
            let room_id = value_as_u64(room_info.pointer("/data/room_id"))
                .unwrap()
                .to_string();
            let uid = value_as_u64(room_info.pointer("/data/uid")).unwrap_or_default();
            eprintln!("room_info: {:?}", checkpoint.elapsed());
            checkpoint = Instant::now();

            let _anchor = client.resolve_anchor(uid, String::new()).await;
            eprintln!("master_info: {:?}", checkpoint.elapsed());
            checkpoint = Instant::now();

            let discovery = client.request_play_info(&room_id, 0).await.unwrap();
            let discovery_playurl = playurl_payload(&discovery).unwrap();
            let highest_qn = highest_available_avc_qn(discovery_playurl).unwrap_or(10_000);
            eprintln!("play_info_discovery: {:?}", checkpoint.elapsed());
            checkpoint = Instant::now();

            let selected = client
                .request_play_info(&room_id, highest_qn)
                .await
                .unwrap();
            let playurl = playurl_payload(&selected).unwrap();
            let candidate = extract_play_candidates(playurl, highest_qn)
                .unwrap()
                .into_iter()
                .next()
                .unwrap();
            eprintln!(
                "play_info_selected: {:?} ({})",
                checkpoint.elapsed(),
                quality_label(playurl, candidate.current_qn)
            );
            checkpoint = Instant::now();

            let mut response = client.http.get(&candidate.url).send().await.unwrap();
            eprintln!(
                "media_headers: {:?} ({})",
                checkpoint.elapsed(),
                response.status()
            );
            checkpoint = Instant::now();
            let first_chunk = response.chunk().await.unwrap().unwrap();
            eprintln!(
                "media_first_chunk: {:?} ({} bytes)",
                checkpoint.elapsed(),
                first_chunk.len()
            );
            let buffering_started = Instant::now();
            let mut buffered = first_chunk.len();
            let mut reported_256k = false;
            while buffered < 768 * 1_024 {
                let Some(chunk) = response.chunk().await.unwrap() else {
                    break;
                };
                buffered += chunk.len();
                if !reported_256k && buffered >= 256 * 1_024 {
                    eprintln!("media_256k: {:?}", buffering_started.elapsed());
                    reported_256k = true;
                }
            }
            eprintln!("media_768k: {:?}", buffering_started.elapsed());
            eprintln!("metadata_total: {:?}", started.elapsed());
            assert_eq!(
                value_as_i64(room_info.pointer("/data/live_status")),
                Some(1)
            );
        });
    }

    #[test]
    #[ignore = "measures chunk gaps on current Bilibili FLV lines"]
    fn measures_current_bilibili_flv_chunk_gaps() {
        tauri::async_runtime::block_on(async {
            let client = BilibiliClient::new().unwrap();
            let room = std::env::var("SIMPLE_LIVE_BILIBILI_TEST_ROOM")
                .unwrap_or_else(|_| "5050".to_string());
            let status = client.auth.status(&client.http).await.unwrap();
            eprintln!("authenticated={}", status.authenticated);

            let room_info = client
                .get_public_json(BILIBILI_ROOM_INFO_API, &[("room_id", room.as_str())])
                .await
                .unwrap();
            assert_eq!(
                value_as_i64(room_info.pointer("/data/live_status")),
                Some(1),
                "the integration-test room is not live"
            );
            let room_id = value_as_u64(room_info.pointer("/data/room_id"))
                .unwrap()
                .to_string();
            let discovery = client.request_play_info(&room_id, 0).await.unwrap();
            let highest_qn =
                highest_available_avc_qn(playurl_payload(&discovery).unwrap()).unwrap_or(10_000);
            let selected = client
                .request_play_info(&room_id, highest_qn)
                .await
                .unwrap();
            let candidates =
                extract_play_candidates(playurl_payload(&selected).unwrap(), highest_qn).unwrap();
            assert!(!candidates.is_empty());

            let mut tasks = Vec::new();
            for (index, candidate) in candidates.into_iter().take(4).enumerate() {
                let http = client.http.clone();
                tasks.push(tauri::async_runtime::spawn(async move {
                    let protocol = candidate.protocol.clone();
                    let format = candidate.format.clone();
                    let mut response = http
                        .get(&candidate.url)
                        .timeout(Duration::from_secs(135))
                        .send()
                        .await
                        .unwrap();
                    assert!(response.status().is_success());

                    let started = Instant::now();
                    let mut previous = started;
                    let mut bytes = 0usize;
                    let mut chunks = 0usize;
                    let mut gaps_over_one_second = 0usize;
                    let mut gaps_over_two_seconds = 0usize;
                    let mut maximum_gap = Duration::ZERO;
                    let mut eof = false;
                    while started.elapsed() < Duration::from_secs(120) {
                        match response.chunk().await.unwrap() {
                            Some(chunk) => {
                                let now = Instant::now();
                                let gap = now.duration_since(previous);
                                maximum_gap = maximum_gap.max(gap);
                                gaps_over_one_second += usize::from(gap >= Duration::from_secs(1));
                                gaps_over_two_seconds += usize::from(gap >= Duration::from_secs(2));
                                bytes += chunk.len();
                                chunks += 1;
                                previous = now;
                            }
                            None => {
                                eof = true;
                                break;
                            }
                        }
                    }
                    (
                        index,
                        candidate.line_name,
                        protocol,
                        format,
                        candidate.current_qn,
                        started.elapsed(),
                        bytes,
                        chunks,
                        maximum_gap,
                        gaps_over_one_second,
                        gaps_over_two_seconds,
                        eof,
                    )
                }));
            }

            for task in tasks {
                let (
                    index,
                    line,
                    protocol,
                    format,
                    qn,
                    elapsed,
                    bytes,
                    chunks,
                    maximum_gap,
                    gaps_over_one_second,
                    gaps_over_two_seconds,
                    eof,
                ) = task.await.unwrap();
                eprintln!(
                    "line {index} {line}: protocol={protocol}, format={format}, qn={qn}, elapsed={elapsed:?}, bytes={bytes}, chunks={chunks}, max_gap={maximum_gap:?}, gaps>=1s={gaps_over_one_second}, gaps>=2s={gaps_over_two_seconds}, eof={eof}"
                );
            }
        });
    }

    #[test]
    #[ignore = "inspects current Bilibili HLS manifest timing"]
    fn inspects_current_bilibili_hls_manifests() {
        tauri::async_runtime::block_on(async {
            let client = BilibiliClient::new().unwrap();
            let room = std::env::var("SIMPLE_LIVE_BILIBILI_TEST_ROOM")
                .unwrap_or_else(|_| "5050".to_string());
            let _ = client.auth.status(&client.http).await.unwrap();
            let room_info = client
                .get_public_json(BILIBILI_ROOM_INFO_API, &[("room_id", room.as_str())])
                .await
                .unwrap();
            let room_id = value_as_u64(room_info.pointer("/data/room_id"))
                .unwrap()
                .to_string();
            let discovery = client.request_play_info(&room_id, 0).await.unwrap();
            let highest_qn =
                highest_available_avc_qn(playurl_payload(&discovery).unwrap()).unwrap_or(10_000);
            let selected = client
                .request_play_info(&room_id, highest_qn)
                .await
                .unwrap();
            let mut hls_playurl = playurl_payload(&selected).unwrap().clone();
            if let Some(streams) = hls_playurl.get_mut("stream").and_then(Value::as_array_mut) {
                streams.retain(|stream| {
                    string_value(stream.get("protocol_name")).eq_ignore_ascii_case("http_hls")
                });
            }
            let candidates = extract_play_candidates(&hls_playurl, highest_qn).unwrap();
            let hls = candidates
                .into_iter()
                .filter(|candidate| candidate.protocol.eq_ignore_ascii_case("http_hls"))
                .take(3)
                .collect::<Vec<_>>();
            assert!(!hls.is_empty(), "the room did not return an HLS candidate");
            for (index, candidate) in hls.into_iter().enumerate() {
                let manifest = client
                    .http
                    .get(&candidate.url)
                    .send()
                    .await
                    .unwrap()
                    .text()
                    .await
                    .unwrap();
                let durations = manifest
                    .lines()
                    .filter_map(|line| line.strip_prefix("#EXTINF:"))
                    .filter_map(|value| value.trim_end_matches(',').parse::<f64>().ok())
                    .collect::<Vec<_>>();
                let total = durations.iter().sum::<f64>();
                let target = manifest
                    .lines()
                    .find_map(|line| line.strip_prefix("#EXT-X-TARGETDURATION:"));
                eprintln!(
                    "hls {index} {}: format={}, segments={}, listed={total:.2}s, target={target:?}",
                    candidate.line_name,
                    candidate.format,
                    durations.len(),
                );
            }
        });
    }
}
