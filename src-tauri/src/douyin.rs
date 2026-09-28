//! Douyin page-data parsing, following DouyinLiveRecorder's roomStore and stream maps.
use crate::{
    live_source::{self as common, Candidate},
    stream::LiveStream,
};
use reqwest::Client;
use serde_json::Value;

const HOSTS: &[&str] = &[
    "live.douyin.com",
    "v.douyin.com",
    "www.douyin.com",
    "douyin.com",
    "webcast.amemv.com",
    "webcast.iesdouyin.com",
    "www.iesdouyin.com",
];
const CDN: &[&str] = &[
    "douyincdn.com",
    "bytecdn.cn",
    "byteimg.com",
    "bytedance.com",
    "pstatp.com",
    "amemv.com",
    "douyin.com",
];
pub struct DouyinClient {
    http: Client,
}
impl DouyinClient {
    pub fn new() -> Result<Self, String> {
        Ok(Self {
            http: common::client("https://live.douyin.com/")?,
        })
    }

    async fn fetch_room(&self, source: &str) -> Result<(String, Value), String> {
        let mut url = common::source_url(source, HOSTS)?;
        let mut cookie = String::new();
        let mut attempted_api = false;
        for _ in 0..5 {
            if !attempted_api && url.host_str() == Some("live.douyin.com") {
                attempted_api = true;
                let id = common::room_slug(&url)?;
                if let Ok(room) = self.api_room(&id, None).await {
                    return Ok((id, room));
                }
            }
            if let Some((id, sec_user)) = share_room(&url) {
                return self
                    .api_room(&id, Some(&sec_user))
                    .await
                    .map(|room| (id, room));
            }
            let mut request = self.http.get(url.clone());
            if url.host_str() == Some("live.douyin.com") && !cookie.is_empty() {
                request = request.header(reqwest::header::COOKIE, &cookie);
            }
            let response = request.send().await.map_err(|_| "抖音直播间连接失败")?;
            if response.status().is_redirection() {
                let location = response
                    .headers()
                    .get(reqwest::header::LOCATION)
                    .and_then(|v| v.to_str().ok())
                    .ok_or("抖音分享链接跳转失败")?;
                let next = url.join(location).map_err(|_| "抖音分享链接无效")?;
                url = common::source_url(next.as_str(), HOSTS)?;
                continue;
            }
            let nonce = response
                .headers()
                .get_all(reqwest::header::SET_COOKIE)
                .iter()
                .filter_map(|v| v.to_str().ok())
                .filter_map(|v| v.split(';').next())
                .find(|v| v.starts_with("__ac_nonce="))
                .map(str::to_owned);
            let html = common::body(Ok(response)).await?;
            if let Some(room) = parse_page(&html) {
                let id = if url.host_str() == Some("live.douyin.com") {
                    common::room_slug(&url)?
                } else {
                    common::text(&room["id_str"])
                };
                return Ok((id, room));
            }
            if cookie.is_empty() {
                if let Some(nonce) = nonce {
                    cookie = nonce;
                    continue;
                }
            }
            return Err(
                "抖音未返回房间资料，可能触发验证；请使用 live.douyin.com 的直播间链接或稍后重试"
                    .into(),
            );
        }
        Err("抖音分享链接跳转次数过多".into())
    }
    async fn api_room(&self, id: &str, sec_user: Option<&str>) -> Result<Value, String> {
        // A fresh anonymous visitor cookie is scoped to Douyin requests only.
        let register = self.http.post("https://ttwid.bytedance.com/ttwid/union/register/")
            .json(&serde_json::json!({"region":"cn","aid":1768,"needFid":false,"service":"www.ixigua.com","migrate_info":{"ticket":"","source":"node"},"cbUrlProtocol":"https","union":true})).send().await;
        let cookie = register
            .ok()
            .and_then(|response| {
                response
                    .headers()
                    .get_all(reqwest::header::SET_COOKIE)
                    .iter()
                    .filter_map(|v| v.to_str().ok())
                    .filter_map(|v| v.split(';').next())
                    .find(|v| v.starts_with("ttwid="))
                    .map(str::to_owned)
            })
            .unwrap_or_default();
        let mut url = url::Url::parse("https://live.douyin.com/webcast/room/web/enter/").unwrap();
        url.query_pairs_mut().extend_pairs([
            ("aid", "6383"),
            ("app_name", "douyin_web"),
            ("live_id", "1"),
            ("device_platform", "web"),
            ("language", "zh-CN"),
            ("browser_language", "zh-CN"),
            ("browser_platform", "Win32"),
            ("browser_name", "Chrome"),
            ("browser_version", "141.0.0.0"),
            ("web_rid", id),
            ("msToken", ""),
        ]);
        if let Some(sec_user) = sec_user {
            url = url::Url::parse("https://webcast.amemv.com/webcast/room/reflow/info/").unwrap();
            url.query_pairs_mut().extend_pairs([
                ("type_id", "0"),
                ("live_id", "1"),
                ("room_id", id),
                ("sec_user_id", sec_user),
                ("version_code", "99.99.99"),
                ("app_id", "1128"),
            ]);
        }
        let time = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        let signature =
            crate::douyin_sign::sign(url.query().unwrap_or_default(), common::USER_AGENT, time);
        url.query_pairs_mut().append_pair("a_bogus", &signature);
        let mut request = self.http.get(url);
        if !cookie.is_empty() {
            request = request.header(reqwest::header::COOKIE, cookie);
        }
        let payload = common::json(request.send().await).await?;
        let mut room = if sec_user.is_some() {
            payload["data"]["room"].clone()
        } else {
            payload["data"]["data"][0].clone()
        };
        if room.get("status").is_none() {
            return Err("抖音未返回房间状态".into());
        }
        if room.get("owner").is_none() {
            room["owner"] = payload["data"]["user"].clone();
        }
        Ok(room)
    }
    pub async fn is_live(&self, source: &str) -> Result<bool, String> {
        let (_, room) = self.fetch_room(source).await?;
        live_status(&room)
    }
    pub async fn resolve(
        &self,
        source: &str,
        line: usize,
        quality: u32,
    ) -> Result<LiveStream, String> {
        let (id, data) = self.fetch_room(source).await?;
        let mut stream = common::room("douyin", "抖音", id, source.into());
        stream.title = common::text(&data["title"]);
        stream.anchor = common::text(&data["owner"]["nickname"]);
        stream.avatar_url = common::text(&data["owner"]["avatar_thumb"]["url_list"][0]);
        stream.cover_url = common::text(&data["cover"]["url_list"][0]);
        stream.is_live = live_status(&data)?;
        if stream.is_live {
            common::select(&mut stream, &candidates(&data), line, quality)?;
        }
        Ok(stream)
    }
}

fn share_room(url: &url::Url) -> Option<(String, String)> {
    if !url.path().contains("/reflow/") {
        return None;
    }
    let id = url.path_segments()?.filter(|s| !s.is_empty()).next_back()?;
    let sec_user = url
        .query_pairs()
        .find(|(k, _)| k == "sec_user_id")?
        .1
        .into_owned();
    if id.is_empty()
        || id.len() > 20
        || !id.bytes().all(|b| b.is_ascii_digit())
        || sec_user.is_empty()
        || sec_user.len() > 256
    {
        return None;
    }
    Some((id.into(), sec_user))
}

fn live_status(room: &Value) -> Result<bool, String> {
    match common::number(&room["status"]) {
        Some(2) => Ok(true),
        Some(4) => Ok(false),
        _ => Err("抖音房间直播状态未知".into()),
    }
}
fn room_from_text(input: &str) -> Option<Value> {
    let mut remaining = input;
    while let Some(index) = remaining.find("\"roomStore\":") {
        let store = common::json_after(&remaining[index..], "\"roomStore\":")?;
        let room = &store["roomInfo"]["room"];
        if room.get("status").is_some() {
            let mut room = room.clone();
            if room.get("owner").is_none() {
                room["owner"] = store["roomInfo"]["anchor"].clone();
            }
            return Some(room);
        }
        remaining = &remaining[index + 12..];
    }
    None
}
fn parse_page(html: &str) -> Option<Value> {
    if let Some(room) = room_from_text(html) {
        return Some(room);
    }
    // Next.js serializes page state inside JS string literals. Decode each literal
    // with JSON, retaining escapes inside nested stream_data and broadcaster titles.
    let strings = regex::Regex::new(r#""(?:\\.|[^"\\])*""#).ok()?;
    for token in strings.find_iter(html) {
        if !token.as_str().contains("roomStore") {
            continue;
        }
        if let Ok(decoded) = serde_json::from_str::<String>(token.as_str()) {
            if let Some(room) = room_from_text(&decoded) {
                return Some(room);
            }
        }
    }
    None
}
fn candidates(room: &Value) -> Vec<Candidate> {
    let mut result = Vec::new();
    let stream = &room["stream_url"];
    // Stable values are quality identifiers, never measured bitrate.
    let levels = [
        ("ORIGIN", "origin", 0, "原画"),
        ("FULL_HD1", "uhd", 1, "蓝光"),
        ("HD1", "hd", 2, "超清"),
        ("SD1", "sd", 3, "高清"),
        ("SD2", "ld", 4, "标清"),
    ];
    let core = stream
        .pointer("/live_core_sdk_data/pull_data/stream_data")
        .and_then(Value::as_str)
        .and_then(|s| serde_json::from_str::<Value>(s).ok())
        .unwrap_or(Value::Null);
    for (key, core_key, quality, label) in levels {
        let main = &core["data"][core_key]["main"];
        let codec = main["sdk_params"]
            .as_str()
            .and_then(|s| serde_json::from_str::<Value>(s).ok())
            .map(|v| common::text(&v["VCodec"]).to_lowercase())
            .unwrap_or_default();
        for (format, map) in [("flv", "flv_pull_url"), ("hls", "hls_pull_url_map")] {
            let raw = if matches!(codec.as_str(), "h265" | "hevc") {
                ""
            } else {
                main[format].as_str().unwrap_or_default()
            };
            let raw = if raw.is_empty() {
                stream[map][key].as_str().unwrap_or_default()
            } else {
                raw
            };
            if let Some(url) = common::media_url(raw, CDN) {
                if !result.iter().any(|c: &Candidate| c.url == url) {
                    result.push(Candidate {
                        url,
                        format: format.into(),
                        quality,
                        label: label.into(),
                    });
                }
            }
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn decodes_page_without_corrupting_escaped_titles() {
        let room = json!({"status":2,"title":"主播说\"你好\"\\音乐","owner":{"nickname":"主播"}});
        let state = json!({"roomStore":{"roomInfo":{"room":room}}}).to_string();
        let html = format!(
            "<script>self.__next_f.push([1,{}])</script>",
            serde_json::to_string(&state).unwrap()
        );
        assert_eq!(parse_page(&html).unwrap()["title"], room["title"]);
        assert!(parse_page("verification required").is_none());
    }
    #[test]
    fn orders_qualities_and_rejects_non_platform_streams() {
        let room = json!({"stream_url":{"flv_pull_url":{"HD1":"https://pull.douyincdn.com/hd.flv", "ORIGIN":"https://pull.douyincdn.com/origin.flv", "SD1":"https://evil.test/a.flv"}, "hls_pull_url_map":{"ORIGIN":"https://pull.douyincdn.com/origin.m3u8"}}});
        let options = candidates(&room);
        assert_eq!(options.len(), 3);
        let mut stream = common::room("douyin", "抖音", "1".into(), "".into());
        common::select(&mut stream, &options, 1, 0).unwrap();
        assert_eq!(stream.format, "hls");
        assert_eq!(stream.quality_label, "原画");
        common::select(&mut stream, &options, 0, 2).unwrap();
        assert_eq!(stream.quality_label, "超清");
    }
    #[test]
    fn extracts_shared_live_room_without_accepting_video_links() {
        let url = url::Url::parse(
            "https://webcast.amemv.com/douyin/webcast/reflow/123456?sec_user_id=public_id",
        )
        .unwrap();
        assert_eq!(
            share_room(&url),
            Some(("123456".into(), "public_id".into()))
        );
        assert!(
            share_room(&url::Url::parse("https://www.douyin.com/video/123").unwrap()).is_none()
        );
    }
    #[test]
    fn missing_status_is_not_offline() {
        assert!(live_status(&json!({})).is_err());
        assert!(!live_status(&json!({"status":4})).unwrap());
    }
}
