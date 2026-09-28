use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::{engine::general_purpose::STANDARD, Engine as _};
use reqwest::{
    header::{HeaderMap, HeaderValue, ACCEPT, ORIGIN, REFERER, USER_AGENT},
    Client,
};
use serde_json::Value;
use url::{form_urlencoded, Url};

use crate::huya_wup::resolve_replay;
use crate::stream::{LiveStream, StreamLineOption, StreamQualityOption};

const HUYA_ROOM_API: &str = "https://mp.huya.com/cache.php";
const HUYA_ORIGIN: &str = "https://www.huya.com";
const HUYA_USER_AGENT: &str =
    "Mozilla/5.0 (Linux; Android 11; Pixel 5) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/90.0.4430.91 Mobile Safari/537.36";
const MAX_BITRATE_KBPS: u32 = 50_000;

pub struct HuyaClient {
    http: Client,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RawLine {
    cdn_type: String,
    base_url: String,
    anti_code: String,
    stream_name: String,
    presenter_uid: u64,
    is_flv: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RoomPlayback {
    Live,
    Replay,
    Offline,
}

impl HuyaClient {
    pub fn new() -> Result<Self, String> {
        let mut headers = HeaderMap::new();
        headers.insert(ACCEPT, HeaderValue::from_static("*/*"));
        headers.insert(ORIGIN, HeaderValue::from_static(HUYA_ORIGIN));
        headers.insert(REFERER, HeaderValue::from_static("https://www.huya.com/"));
        headers.insert(USER_AGENT, HeaderValue::from_static(HUYA_USER_AGENT));

        let http = Client::builder()
            .default_headers(headers)
            .connect_timeout(Duration::from_secs(8))
            .timeout(Duration::from_secs(15))
            .tcp_keepalive(Duration::from_secs(30))
            .pool_idle_timeout(Duration::from_secs(90))
            .pool_max_idle_per_host(2)
            .redirect(reqwest::redirect::Policy::limited(3))
            .build()
            .map_err(|error| format!("无法初始化网络客户端：{error}"))?;

        Ok(Self { http })
    }

    async fn room_info(&self, room_id: &str) -> Result<Value, String> {
        let mut endpoint = Url::parse(HUYA_ROOM_API).map_err(|error| error.to_string())?;
        endpoint.query_pairs_mut().extend_pairs([
            ("m", "Live"),
            ("do", "profileRoom"),
            ("roomid", room_id),
            ("showSecret", "1"),
        ]);

        let response = self
            .http
            .get(endpoint)
            .send()
            .await
            .map_err(|error| format!("连接虎牙失败：{error}"))?;

        if !response.status().is_success() {
            return Err(format!("虎牙接口返回状态 {}", response.status()));
        }

        let payload = response
            .json::<Value>()
            .await
            .map_err(|error| format!("虎牙返回了无法识别的数据：{error}"))?;

        let status = payload
            .get("status")
            .and_then(value_as_i64)
            .unwrap_or_default();
        if status != 200 {
            return Err(format!("虎牙未返回有效房间信息（状态 {status}）"));
        }

        Ok(payload)
    }

    pub async fn is_live(&self, source: &str) -> Result<bool, String> {
        let room_id = parse_room_id(source)?;
        let payload = self.room_info(&room_id).await?;
        let status = payload.pointer("/data/liveStatus").and_then(Value::as_str)
            .ok_or_else(|| "虎牙房间直播状态缺失".to_string())?;
        match status.trim().to_ascii_uppercase().as_str() {
            "ON" => Ok(true),
            "OFF" | "REPLAY" => Ok(false),
            _ => Err("虎牙房间直播状态未知".to_string()),
        }
    }

    pub async fn resolve(
        &self,
        source: &str,
        requested_line_index: usize,
        bitrate: u32,
    ) -> Result<LiveStream, String> {
        if bitrate > MAX_BITRATE_KBPS {
            return Err("播放码率超出允许范围".to_string());
        }

        let room_id = parse_room_id(source)?;
        let payload = self.room_info(&room_id).await?;

        let data = payload
            .get("data")
            .ok_or_else(|| "虎牙房间信息为空".to_string())?;
        let live_state = data
            .get("liveStatus")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_ascii_uppercase();
        let title = string_at(data, &["liveData", "introduction"]);
        let anchor = string_at(data, &["profileInfo", "nick"]);
        let avatar_url = non_empty_or(
            string_at(data, &["profileInfo", "avatar180"]),
            string_at(data, &["liveData", "avatar180"]),
        );
        let cover_url = non_empty_or(
            string_at(data, &["liveData", "screenshot"]),
            string_at(data, &["liveData", "gameCover"]),
        );
        let canonical_room = string_at(data, &["liveData", "profileRoom"]);
        let room_id = if canonical_room.is_empty() {
            room_id
        } else {
            canonical_room
        };
        let playback = room_playback(&live_state);
        let is_live = playback == RoomPlayback::Live;

        if playback == RoomPlayback::Replay {
            let fallback_url = extract_replay_url(data)?;
            let fallback_sync_time = data
                .pointer("/liveData/videoSyncTime")
                .and_then(value_as_u64)
                .unwrap_or_default() as f64;
            let presenter_uid = data
                .pointer("/profileInfo/uid")
                .and_then(value_as_u64)
                .or_else(|| data.pointer("/liveData/uid").and_then(value_as_u64))
                .unwrap_or_default();
            let replay = resolve_replay(&self.http, presenter_uid).await.ok();
            let url = replay
                .as_ref()
                .map(|source| source.url.clone())
                .or(fallback_url);
            let start_position_seconds = replay
                .as_ref()
                .map(|source| source.sync_time_seconds)
                .unwrap_or(fallback_sync_time);
            let quality_label = replay
                .as_ref()
                .map(|source| source.quality_label.clone())
                .unwrap_or_else(|| "360P · 流畅".to_string());
            let nominal_bitrate = replay
                .as_ref()
                .map(|source| source.nominal_bitrate_kbps)
                .unwrap_or_default();
            return Ok(LiveStream {
                platform: "huya".to_string(),
                platform_label: "虎牙".to_string(),
                source_url: format!("https://www.huya.com/{room_id}"),
                room_id,
                title,
                anchor,
                avatar_url,
                cover_url,
                is_live: false,
                is_replay: true,
                url,
                line_index: 0,
                line_count: 1,
                line_name: "录像".to_string(),
                line_options: vec![StreamLineOption {
                    index: 0,
                    label: "录像回放".to_string(),
                }],
                quality_label,
                quality_options: Vec::new(),
                bitrate: nominal_bitrate,
                start_position_seconds,
                format: "hls".to_string(),
            });
        }

        if !is_live {
            return Ok(LiveStream {
                platform: "huya".to_string(),
                platform_label: "虎牙".to_string(),
                source_url: format!("https://www.huya.com/{room_id}"),
                room_id,
                title,
                anchor,
                avatar_url,
                cover_url,
                is_live: false,
                is_replay: false,
                url: None,
                line_index: 0,
                line_count: 0,
                line_name: String::new(),
                line_options: Vec::new(),
                quality_label: String::new(),
                quality_options: Vec::new(),
                bitrate,
                start_position_seconds: 0.0,
                format: String::new(),
            });
        }

        let lines = extract_live_lines(data);
        if lines.is_empty() {
            return Err("当前直播间没有可用的直播线路".to_string());
        }

        let line_index = requested_line_index % lines.len();
        let line = &lines[line_index];
        let url = build_play_url(line, bitrate, now_millis()?)?;
        let (quality_label, nominal_bitrate) = selected_quality(data, bitrate);
        let line_options = live_line_options(&lines);
        let quality_options = available_qualities(data);

        Ok(LiveStream {
            platform: "huya".to_string(),
            platform_label: "虎牙".to_string(),
            source_url: format!("https://www.huya.com/{room_id}"),
            room_id,
            title,
            anchor,
            avatar_url,
            cover_url,
            is_live: true,
            is_replay: false,
            url: Some(url),
            line_index,
            line_count: lines.len(),
            line_name: line.cdn_type.clone(),
            line_options,
            quality_label,
            quality_options,
            bitrate: nominal_bitrate,
            start_position_seconds: 0.0,
            format: if line.is_flv { "flv" } else { "hls" }.to_string(),
        })
    }
}

fn validated_huya_media_url(value: &str) -> Result<Url, String> {
    let url = Url::parse(value).map_err(|_| "媒体地址格式不正确".to_string())?;
    if url.scheme() != "https" {
        return Err("媒体地址只允许 HTTPS".to_string());
    }
    let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
    if host != "huya.com" && !host.ends_with(".huya.com") {
        return Err("媒体地址只允许虎牙域名".to_string());
    }
    Ok(url)
}

fn parse_room_id(source: &str) -> Result<String, String> {
    let source = source.trim();
    if source.is_empty() {
        return Err("请输入虎牙直播间链接".to_string());
    }

    let normalized = if source.contains("://") {
        source.to_string()
    } else {
        format!("https://{source}")
    };
    let url = Url::parse(&normalized).map_err(|_| "直播间链接格式不正确".to_string())?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err("只支持 http 或 https 的虎牙链接".to_string());
    }

    let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
    if host != "huya.com" && !host.ends_with(".huya.com") {
        return Err("目前只支持 huya.com 的直播间链接".to_string());
    }

    let room_id = url
        .path_segments()
        .and_then(|mut segments| segments.find(|segment| !segment.is_empty()))
        .unwrap_or_default();
    let is_valid = !room_id.is_empty()
        && room_id.len() <= 64
        && room_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'));
    if !is_valid {
        return Err("链接中没有有效的虎牙房间号".to_string());
    }

    Ok(room_id.to_string())
}

fn extract_live_lines(data: &Value) -> Vec<RawLine> {
    let profile_uid = data
        .pointer("/profileInfo/uid")
        .and_then(value_as_u64)
        .unwrap_or_default();
    let mut candidates = data
        .pointer("/stream/baseSteamInfoList")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|item| {
            let flv_base_url = item
                .get("sFlvUrl")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim();
            let flv_anti_code = item
                .get("sFlvAntiCode")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim();
            let (base_url, anti_code, is_flv) =
                if !flv_base_url.is_empty() && !flv_anti_code.is_empty() {
                    (flv_base_url.to_string(), flv_anti_code.to_string(), true)
                } else {
                    (
                        item.get("sHlsUrl")?.as_str()?.trim().to_string(),
                        item.get("sHlsAntiCode")?.as_str()?.trim().to_string(),
                        false,
                    )
                };
            let stream_name = item.get("sStreamName")?.as_str()?.trim().to_string();
            if base_url.is_empty() || anti_code.is_empty() || stream_name.is_empty() {
                return None;
            }
            let presenter_uid = item
                .get("lPresenterUid")
                .and_then(value_as_u64)
                .or_else(|| {
                    stream_name
                        .split('-')
                        .next()
                        .and_then(|part| part.parse().ok())
                })
                .unwrap_or(profile_uid);
            Some(RawLine {
                cdn_type: item
                    .get("sCdnType")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                base_url,
                anti_code,
                stream_name,
                presenter_uid,
                is_flv,
            })
        })
        .collect::<Vec<_>>();

    let reported_priorities = data
        .pointer("/stream/flv/multiLine")
        .or_else(|| data.pointer("/stream/hls/multiLine"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|item| {
            item.get("cdnType")
                .or_else(|| item.get("sCdnType"))
                .and_then(Value::as_str)
        })
        .collect::<Vec<_>>();

    let mut priorities = vec!["HS"];
    priorities.extend(
        reported_priorities
            .into_iter()
            .filter(|cdn_type| !cdn_type.eq_ignore_ascii_case("HS")),
    );

    let mut ordered = Vec::with_capacity(candidates.len());
    for cdn_type in priorities {
        if let Some(index) = candidates
            .iter()
            .position(|line| line.cdn_type.eq_ignore_ascii_case(cdn_type))
        {
            ordered.push(candidates.remove(index));
        }
    }
    ordered.extend(candidates);
    ordered
}

fn extract_replay_url(data: &Value) -> Result<Option<String>, String> {
    let raw = non_empty_or(
        string_at(data, &["liveData", "hlsUrl"]),
        string_at(data, &["liveData", "hls"]),
    );
    if raw.is_empty() {
        return Ok(None);
    }

    let mut url = Url::parse(&raw).map_err(|_| "虎牙回放地址格式不正确".to_string())?;
    if url.scheme() == "http" {
        url.set_scheme("https")
            .map_err(|_| "虎牙回放地址无法升级为 HTTPS".to_string())?;
    }
    validated_huya_media_url(url.as_str())?;
    Ok(Some(url.to_string()))
}

fn build_play_url(line: &RawLine, bitrate: u32, now_ms: u64) -> Result<String, String> {
    let query = form_urlencoded::parse(line.anti_code.as_bytes())
        .into_owned()
        .collect::<std::collections::HashMap<_, _>>();
    let encoded_fm = query
        .get("fm")
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "虎牙播放签名缺少 fm 字段".to_string())?;
    let decoded_fm = decode_base64(encoded_fm)?;
    let secret_prefix = decoded_fm
        .split('_')
        .next()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "虎牙播放签名格式不正确".to_string())?;
    let ctype = query
        .get("ctype")
        .filter(|value| !value.trim().is_empty())
        .map(String::as_str)
        .unwrap_or("huya_live");
    let platform_id = query
        .get("t")
        .filter(|value| !value.trim().is_empty())
        .map(String::as_str)
        .unwrap_or("100");
    let is_wap = platform_id == "103";

    let now_seconds = now_ms / 1000;
    let provided_expiry = query
        .get("wsTime")
        .and_then(|value| u64::from_str_radix(value, 16).ok())
        .ok_or_else(|| "虎牙播放签名缺少有效的 wsTime 字段".to_string())?;
    if provided_expiry <= now_seconds + 5 {
        return Err("虎牙播放令牌已过期".to_string());
    }
    let ws_time = format!("{provided_expiry:x}");
    let uid = line.presenter_uid;
    let converted_uid = rotate_uid(uid);
    let calculation_uid = if is_wap { uid } else { converted_uid };
    let sequence_id = uid.saturating_add(now_ms);
    let secret_hash = md5_hex(format!("{sequence_id}|{ctype}|{platform_id}"));
    let secret = md5_hex(format!(
        "{secret_prefix}_{calculation_uid}_{}_{secret_hash}_{ws_time}",
        line.stream_name
    ));

    let mut output = form_urlencoded::Serializer::new(String::new());
    output.append_pair("wsSecret", &secret);
    output.append_pair("wsTime", &ws_time);
    output.append_pair("seqid", &sequence_id.to_string());
    output.append_pair("ctype", ctype);
    output.append_pair("ver", "1");
    output.append_pair("fs", query.get("fs").map(String::as_str).unwrap_or("bgct"));
    output.append_pair("fm", encoded_fm);
    output.append_pair("t", platform_id);
    if is_wap {
        output.append_pair("uid", &uid.to_string());
        let uuid = (u128::from(now_ms) * 1000 + u128::from(uid)) % u128::from(u32::MAX);
        output.append_pair("uuid", &uuid.to_string());
    } else {
        output.append_pair("u", &converted_uid.to_string());
    }
    output.append_pair("codec", "264");
    if bitrate > 0 {
        output.append_pair("ratio", &bitrate.to_string());
    }
    let signed_query = output.finish();

    let secure_base = if let Some(rest) = line.base_url.strip_prefix("http://") {
        format!("https://{rest}")
    } else if line.base_url.starts_with("https://") {
        line.base_url.clone()
    } else {
        return Err("虎牙返回了不支持的播放协议".to_string());
    };

    let extension = if line.is_flv { "flv" } else { "m3u8" };
    Ok(format!(
        "{}/{}.{extension}?{}",
        secure_base.trim_end_matches('/'),
        line.stream_name,
        signed_query
    ))
}

fn selected_quality(data: &Value, requested_bitrate: u32) -> (String, u32) {
    let source_bitrate = data
        .pointer("/liveData/bitRate")
        .and_then(value_as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .unwrap_or_default();
    let qualities = data
        .pointer("/liveData/bitRateInfo")
        .and_then(Value::as_str)
        .and_then(|raw| serde_json::from_str::<Vec<Value>>(raw).ok())
        .unwrap_or_default();
    let entry = qualities.iter().find(|entry| {
        entry
            .get("iBitRate")
            .and_then(value_as_u64)
            .is_some_and(|value| value == u64::from(requested_bitrate))
    });
    let label = entry
        .and_then(|entry| entry.get("sDisplayName"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|label| !label.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| {
            if requested_bitrate == 0 {
                "原画".to_string()
            } else if requested_bitrate >= 1_000 {
                format!("{}M", requested_bitrate / 1_000)
            } else {
                format!("{requested_bitrate}K")
            }
        });
    let nominal_bitrate = if requested_bitrate == 0 {
        source_bitrate
    } else {
        requested_bitrate
    };
    (label, nominal_bitrate)
}

fn live_line_options(lines: &[RawLine]) -> Vec<StreamLineOption> {
    lines
        .iter()
        .enumerate()
        .map(|(index, line)| StreamLineOption {
            index,
            label: if line.cdn_type.trim().is_empty() {
                format!("线路 {}", index + 1)
            } else {
                format!("{} · 线路 {}", line.cdn_type.trim(), index + 1)
            },
        })
        .collect()
}

fn available_qualities(data: &Value) -> Vec<StreamQualityOption> {
    let mut options = vec![StreamQualityOption {
        value: 0,
        label: "原画".to_string(),
    }];
    let qualities = data
        .pointer("/liveData/bitRateInfo")
        .and_then(Value::as_str)
        .and_then(|raw| serde_json::from_str::<Vec<Value>>(raw).ok())
        .unwrap_or_default();
    for entry in qualities {
        let Some(value) = entry
            .get("iBitRate")
            .and_then(value_as_u64)
            .and_then(|value| u32::try_from(value).ok())
            .filter(|value| *value > 0 && *value <= MAX_BITRATE_KBPS)
        else {
            continue;
        };
        if options.iter().any(|option| option.value == value) {
            continue;
        }
        let label = entry
            .get("sDisplayName")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|label| !label.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| format!("{value} kbps"));
        options.push(StreamQualityOption { value, label });
    }
    options
}

fn non_empty_or(primary: String, fallback: String) -> String {
    if primary.is_empty() {
        fallback
    } else {
        primary
    }
}

fn room_playback(status: &str) -> RoomPlayback {
    if status.eq_ignore_ascii_case("ON") {
        RoomPlayback::Live
    } else if status.eq_ignore_ascii_case("REPLAY") {
        RoomPlayback::Replay
    } else {
        RoomPlayback::Offline
    }
}

fn string_at(value: &Value, path: &[&str]) -> String {
    let mut current = value;
    for key in path {
        let Some(next) = current.get(*key) else {
            return String::new();
        };
        current = next;
    }
    match current {
        Value::String(value) => value.trim().to_string(),
        Value::Number(value) => value.to_string(),
        _ => String::new(),
    }
}

fn value_as_i64(value: &Value) -> Option<i64> {
    value
        .as_i64()
        .or_else(|| value.as_str().and_then(|value| value.parse().ok()))
}

fn value_as_u64(value: &Value) -> Option<u64> {
    value
        .as_u64()
        .or_else(|| value.as_str().and_then(|value| value.parse().ok()))
}

fn decode_base64(value: &str) -> Result<String, String> {
    let mut normalized = value.replace(' ', "+");
    while normalized.len() % 4 != 0 {
        normalized.push('=');
    }
    let bytes = STANDARD
        .decode(normalized)
        .map_err(|_| "虎牙播放签名的 fm 字段无法解码".to_string())?;
    String::from_utf8(bytes).map_err(|_| "虎牙播放签名不是有效文本".to_string())
}

fn rotate_uid(uid: u64) -> u64 {
    let high = uid & !u64::from(u32::MAX);
    high | u64::from((uid as u32).rotate_left(8))
}

fn md5_hex(value: String) -> String {
    format!("{:x}", md5::compute(value.as_bytes()))
}

fn now_millis() -> Result<u64, String> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .map_err(|_| "系统时间不正确，无法生成虎牙播放签名".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::time::Instant;

    #[test]
    fn parses_supported_room_links() {
        assert_eq!(
            parse_room_id("https://www.huya.com/998?from=home").unwrap(),
            "998"
        );
        assert_eq!(parse_room_id("www.huya.com/kaerlol").unwrap(), "kaerlol");
    }

    #[test]
    fn rejects_non_huya_links() {
        assert!(parse_room_id("https://example.com/998").is_err());
        assert!(parse_room_id("https://huya.com/").is_err());
    }

    #[test]
    fn distinguishes_live_replay_and_offline_room_states() {
        assert_eq!(room_playback("ON"), RoomPlayback::Live);
        assert_eq!(room_playback("REPLAY"), RoomPlayback::Replay);
        assert_eq!(room_playback("OFF"), RoomPlayback::Offline);
    }

    #[test]
    fn extracts_replay_hls_and_upgrades_it_to_https() {
        let data = json!({
            "liveData": {
                "hlsUrl": "http://videotx-platform.cdn.huya.com/vhuya/liverecord/demo.m3u8?scene=livereplay"
            }
        });
        assert_eq!(
            extract_replay_url(&data).unwrap().unwrap(),
            "https://videotx-platform.cdn.huya.com/vhuya/liverecord/demo.m3u8?scene=livereplay"
        );
        assert!(extract_replay_url(&json!({
            "liveData": { "hls": "https://example.com/demo.m3u8" }
        }))
        .is_err());
    }

    #[test]
    fn builds_a_fresh_known_signature() {
        let line = RawLine {
            cdn_type: "TX".to_string(),
            base_url: "http://tx.hls.huya.com/src".to_string(),
            anti_code: "wsTime=ffffffff&fm=dGVzdHByZWZpeF8kMF8kMV8kMl8kMw%3D%3D&ctype=tars_mp&fs=bgct&t=102"
                .to_string(),
            stream_name: "12345-demo".to_string(),
            presenter_uid: 12_345,
            is_flv: false,
        };

        let result = build_play_url(&line, 4_000, 1_700_000_000_123).unwrap();
        let url = Url::parse(&result).unwrap();
        let query = url
            .query_pairs()
            .into_owned()
            .collect::<std::collections::HashMap<_, _>>();

        assert_eq!(url.scheme(), "https");
        assert_eq!(
            query.get("wsSecret").unwrap(),
            "a4022750387490976dc30eff88610484"
        );
        assert_eq!(query.get("seqid").unwrap(), "1700000012468");
        assert_eq!(query.get("u").unwrap(), "3160320");
        assert_eq!(query.get("codec").unwrap(), "264");
        assert_eq!(query.get("ratio").unwrap(), "4000");
    }

    #[test]
    fn source_quality_omits_ratio_and_uses_huya_nominal_bitrate() {
        let line = RawLine {
            cdn_type: "TX".to_string(),
            base_url: "http://tx.hls.huya.com/src".to_string(),
            anti_code: "wsTime=ffffffff&fm=dGVzdHByZWZpeF8kMF8kMV8kMl8kMw%3D%3D&ctype=tars_mp&fs=bgct&t=102"
                .to_string(),
            stream_name: "12345-demo".to_string(),
            presenter_uid: 12_345,
            is_flv: false,
        };
        let url = Url::parse(&build_play_url(&line, 0, 1_700_000_000_123).unwrap()).unwrap();
        assert!(!url.query_pairs().any(|(key, _)| key == "ratio"));

        let data = json!({
            "liveData": {
                "bitRate": 10000,
                "bitRateInfo": "[{\"sDisplayName\":\"蓝光10M\",\"iBitRate\":0},{\"sDisplayName\":\"蓝光4M\",\"iBitRate\":4000}]"
            }
        });
        assert_eq!(selected_quality(&data, 0), ("蓝光10M".to_string(), 10_000));
        assert_eq!(
            selected_quality(&data, 4_000),
            ("蓝光4M".to_string(), 4_000)
        );

        let qualities = available_qualities(&data);
        assert_eq!(
            qualities
                .iter()
                .map(|option| (option.value, option.label.as_str()))
                .collect::<Vec<_>>(),
            [(0, "原画"), (4_000, "蓝光4M")]
        );
    }

    #[test]
    fn exposes_huya_lines_in_playback_order() {
        let data = json!({
            "profileInfo": { "uid": 42 },
            "stream": {
                "baseSteamInfoList": [
                    {"sCdnType": "AL", "sFlvUrl": "http://al.flv.huya.com/src", "sFlvAntiCode": "a", "sStreamName": "42-demo"},
                    {"sCdnType": "HS", "sFlvUrl": "http://hs.flv.huya.com/src", "sFlvAntiCode": "b", "sStreamName": "42-demo"}
                ],
                "flv": { "multiLine": [{"cdnType": "HS"}, {"cdnType": "AL"}] }
            }
        });

        let lines = extract_live_lines(&data);
        let options = live_line_options(&lines);
        assert_eq!(
            options
                .iter()
                .map(|option| (option.index, option.label.as_str()))
                .collect::<Vec<_>>(),
            [(0, "HS · 线路 1"), (1, "AL · 线路 2")]
        );
    }

    #[test]
    fn rejects_expired_huya_token_instead_of_inventing_a_new_expiry() {
        let line = RawLine {
            cdn_type: "TX".to_string(),
            base_url: "http://tx.hls.huya.com/src".to_string(),
            anti_code:
                "wsTime=1&fm=dGVzdHByZWZpeF8kMF8kMV8kMl8kMw%3D%3D&ctype=tars_mp&fs=bgct&t=102"
                    .to_string(),
            stream_name: "12345-demo".to_string(),
            presenter_uid: 12_345,
            is_flv: false,
        };

        assert!(build_play_url(&line, 0, 1_700_000_000_123)
            .unwrap_err()
            .contains("已过期"));
    }

    #[test]
    #[ignore = "requires a currently live public Huya room and network access"]
    fn resolves_and_opens_a_fresh_live_flv_stream() {
        tauri::async_runtime::block_on(async {
            let client = HuyaClient::new().unwrap();
            let stream = client
                .resolve("https://www.huya.com/691406", 0, 0)
                .await
                .unwrap();
            assert!(stream.is_live, "the integration-test room is not live");
            assert_eq!(stream.format, "flv");

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
                    .unwrap(),
                "video/x-flv"
            );
        });
    }

    #[test]
    #[ignore = "measures chunk gaps on a current public Huya FLV stream"]
    fn measures_current_huya_flv_chunk_gaps() {
        tauri::async_runtime::block_on(async {
            let client = HuyaClient::new().unwrap();
            let room = std::env::var("SIMPLE_LIVE_HUYA_TEST_ROOM")
                .unwrap_or_else(|_| "691406".to_string());
            let seconds = std::env::var("SIMPLE_LIVE_HUYA_MEASURE_SECONDS")
                .ok()
                .and_then(|value| value.parse::<u64>().ok())
                .unwrap_or(180);
            let line_index = std::env::var("SIMPLE_LIVE_HUYA_TEST_LINE")
                .ok()
                .and_then(|value| value.parse::<usize>().ok())
                .unwrap_or(0);
            let stream = client
                .resolve(&format!("https://www.huya.com/{room}"), line_index, 0)
                .await
                .unwrap();
            assert!(stream.is_live, "the measurement room is not live");
            eprintln!(
                "huya_flv_line={} {}/{} target={}s",
                stream.line_name,
                stream.line_index + 1,
                stream.line_count,
                seconds,
            );

            // Do not reuse HuyaClient's 15 second metadata timeout for a long
            // media body. The actual app's media connection is owned by
            // WKWebView/mpegts.js and has no Rust request timeout.
            let media_http = Client::builder()
                .connect_timeout(Duration::from_secs(8))
                .tcp_keepalive(Duration::from_secs(30))
                .build()
                .unwrap();
            let mut response = media_http
                .get(stream.url.as_deref().unwrap())
                .send()
                .await
                .unwrap();
            assert!(response.status().is_success());

            let started = Instant::now();
            let mut last_chunk_at = started;
            let mut maximum_gap = Duration::ZERO;
            let mut gaps_over_one_second = 0_u32;
            let mut bytes = 0_usize;
            let mut reached_eof = false;
            while started.elapsed() < Duration::from_secs(seconds) {
                let Some(chunk) = response.chunk().await.unwrap() else {
                    reached_eof = true;
                    break;
                };
                let now = Instant::now();
                let gap = now.duration_since(last_chunk_at);
                maximum_gap = maximum_gap.max(gap);
                if gap >= Duration::from_secs(1) {
                    gaps_over_one_second += 1;
                }
                last_chunk_at = now;
                bytes += chunk.len();
            }

            eprintln!(
                "huya_flv_duration={:.1}s bytes={} max_chunk_gap={:?} gaps_ge_1s={} eof={}",
                started.elapsed().as_secs_f64(),
                bytes,
                maximum_gap,
                gaps_over_one_second,
                reached_eof,
            );
            assert!(bytes > 1_024 * 1_024, "too little FLV data was received");
        });
    }

    #[test]
    fn prefers_hs_live_flv_then_keeps_reported_fallbacks() {
        let data = json!({
            "profileInfo": { "uid": 42 },
            "stream": {
                "baseSteamInfoList": [
                    {"sCdnType": "AL", "sFlvUrl": "http://al.flv.huya.com/src", "sFlvAntiCode": "a", "sStreamName": "42-demo"},
                    {"sCdnType": "TX", "sFlvUrl": "http://tx.flv.huya.com/src", "sFlvAntiCode": "b", "sStreamName": "42-demo"},
                    {"sCdnType": "HS", "sFlvUrl": "http://hs.flv.huya.com/src", "sFlvAntiCode": "c", "sStreamName": "42-demo"}
                ],
                "flv": { "multiLine": [{"cdnType": "TX"}, {"cdnType": "HS"}] }
            }
        });

        let lines = extract_live_lines(&data);
        assert!(lines.iter().all(|line| line.is_flv));
        assert_eq!(
            lines
                .iter()
                .map(|line| line.cdn_type.as_str())
                .collect::<Vec<_>>(),
            ["HS", "TX", "AL"]
        );
    }
}
