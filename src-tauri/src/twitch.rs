//! Twitch public-channel playback via the anonymous web GraphQL + Usher HLS path.
//!
//! These are Twitch's web-client interfaces, not a documented third-party API,
//! so every failure is classified instead of being reported as "offline".
use crate::{
    live_source::{self as common, Candidate},
    stream::LiveStream,
};
use reqwest::Client;
use serde_json::{json, Value};
use url::Url;

const HOSTS: &[&str] = &["twitch.tv", "www.twitch.tv", "m.twitch.tv"];
const CLIENT_ID: &str = "kimne78kx3ncx6brgo4mv6wki5h1ko";
const GQL: &str = "https://gql.twitch.tv/gql";
const PLAYBACK_TOKEN_HASH: &str = "ed230aa1e33e07eebb8928504583da78a5173989fadfb1ac94be06a04f3cdbe9";
const CHANNEL_SHELL_HASH: &str = "fea4573a7bf2644f5b3f2cbbdcbee0d17312e48d2e55f080589d053aad353f11";
const STREAM_METADATA_HASH: &str = "b57f9b910f8cd1a4659d894fe7550ccc81ec9052c01e438b290fd66a040b9b93";
/// Hosts the WebView-facing playlist IPC may fetch.
const PLAYLIST_DOMAINS: &[&str] = &["ttvnw.net", "live-video.net"];
const MAX_PLAYLIST_BYTES: usize = 1024 * 1024;

pub struct TwitchClient {
    http: Client,
}

#[derive(Debug, Clone, PartialEq)]
struct Variant {
    url: String,
    height: u32,
    frame_rate: u32,
}

impl TwitchClient {
    pub fn new() -> Result<Self, String> {
        Ok(Self { http: common::client("https://player.twitch.tv/")? })
    }

    pub fn channel(source: &str) -> Result<String, String> {
        let url = common::source_url(source, HOSTS)?;
        let login = common::room_slug(&url).map_err(|_| "链接中没有有效的 Twitch 频道")?;
        if !valid_login(&login) {
            return Err("链接中没有有效的 Twitch 频道".into());
        }
        Ok(login.to_ascii_lowercase())
    }

    async fn gql(&self, operation: &str, hash: &str, variables: Value) -> Result<Value, String> {
        let body = json!({
            "operationName": operation,
            "variables": variables,
            "extensions": { "persistedQuery": { "version": 1, "sha256Hash": hash } },
        });
        let response = common::json(
            self.http
                .post(GQL)
                .header("Client-ID", CLIENT_ID)
                .header(reqwest::header::ORIGIN, "https://player.twitch.tv")
                .json(&body)
                .send()
                .await,
        )
        .await?;
        if let Some(message) = response.get("errors").and_then(|e| e.get(0)).and_then(|e| e.get("message")) {
            return Err(format!("Twitch 接口返回错误：{}", message.as_str().unwrap_or("未知")));
        }
        Ok(response)
    }

    /// `Ok(None)` is a channel that does not exist; transport problems are `Err`.
    async fn shell(&self, login: &str) -> Result<Option<Value>, String> {
        let response = self.gql("ChannelShell", CHANNEL_SHELL_HASH, json!({ "login": login })).await?;
        let user = &response["data"]["userOrError"];
        if user.get("login").is_none() {
            return Ok(None);
        }
        Ok(Some(user.clone()))
    }

    pub async fn is_live(&self, source: &str) -> Result<bool, String> {
        let login = Self::channel(source)?;
        let shell = self.shell(&login).await?.ok_or("Twitch 频道不存在")?;
        Ok(!shell["stream"].is_null())
    }

    async fn access_token(&self, login: &str) -> Result<(String, String), String> {
        let response = self
            .gql(
                "PlaybackAccessToken",
                PLAYBACK_TOKEN_HASH,
                json!({
                    "isLive": true, "login": login, "isVod": false, "vodID": "",
                    "playerType": "embed", "platform": "site",
                }),
            )
            .await?;
        let token = &response["data"]["streamPlaybackAccessToken"];
        let signature = common::text(&token["signature"]);
        let value = common::text(&token["value"]);
        if signature.is_empty() || value.is_empty() {
            return Err("Twitch 没有返回播放令牌，频道可能需要登录或受地区限制".into());
        }
        Ok((signature, value))
    }

    async fn master(&self, login: &str) -> Result<String, String> {
        let (signature, token) = self.access_token(login).await?;
        let mut url = Url::parse(&format!("https://usher.ttvnw.net/api/v2/channel/hls/{login}.m3u8"))
            .map_err(|_| "Twitch 播放地址无效")?;
        url.query_pairs_mut()
            .append_pair("platform", "web")
            .append_pair("allow_source", "true")
            .append_pair("allow_audio_only", "true")
            .append_pair("playlist_include_framerate", "true")
            .append_pair("supported_codecs", "h264")
            .append_pair("p", &(rand_cache_buster()).to_string())
            .append_pair("sig", &signature)
            .append_pair("token", &token);
        let response = self.http.get(url).send().await;
        match response {
            Ok(response) if response.status() == reqwest::StatusCode::NOT_FOUND => {
                Err("Twitch 频道当前没有直播".into())
            }
            Ok(response) if response.status() == reqwest::StatusCode::FORBIDDEN => {
                Err("Twitch 拒绝了播放请求，频道可能受地区或订阅限制".into())
            }
            other => common::body(other).await,
        }
    }

    pub async fn resolve(&self, source: &str, line: usize, quality: u32) -> Result<LiveStream, String> {
        let login = Self::channel(source)?;
        let canonical = format!("https://www.twitch.tv/{login}");
        let mut stream = common::room("twitch", "Twitch", login.clone(), canonical);
        let shell = self.shell(&login).await?.ok_or("Twitch 频道不存在")?;
        stream.anchor = common::text(&shell["displayName"]);
        if stream.anchor.is_empty() {
            stream.anchor = login.clone();
        }
        stream.avatar_url = common::text(&shell["profileImageURL"]);
        if shell["stream"].is_null() {
            stream.title = "直播暂未开始".into();
            return Ok(stream);
        }
        stream.is_live = true;
        if let Ok(meta) = self
            .gql("StreamMetadata", STREAM_METADATA_HASH, json!({ "channelLogin": login, "includeIsDJ": true }))
            .await
        {
            stream.title = common::text(&meta["data"]["user"]["lastBroadcast"]["title"]);
        }

        let master = match self.master(&login).await {
            Ok(master) => master,
            Err(message) if message.contains("没有直播") => {
                stream.is_live = false;
                stream.title = "直播暂未开始".into();
                return Ok(stream);
            }
            Err(message) => return Err(message),
        };
        let variants = parse_master(&master)?;
        let candidates: Vec<Candidate> = variants
            .iter()
            .map(|variant| Candidate {
                url: variant.url.clone(),
                format: "hls".into(),
                quality: quality_key(variant),
                label: quality_label(variant),
            })
            .collect();
        common::select(&mut stream, &candidates, line, quality)?;
        stream.bitrate = 0;
        Ok(stream)
    }
}

/// Fetches one Twitch HLS playlist for the WebView. Windows and Android serve
/// the app from `http://tauri.localhost`, which Twitch's playlist CDN answers
/// with 403; Rust has no such Origin. Text playlists only; segments stay direct.
pub async fn fetch_playlist(url: &str) -> Result<String, String> {
    let parsed = Url::parse(url).map_err(|_| "Twitch 播放列表地址无效")?;
    let host = parsed.host_str().unwrap_or_default();
    if parsed.scheme() != "https"
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.port().is_some()
        || !PLAYLIST_DOMAINS.iter().any(|d| host == *d || host.ends_with(&format!(".{d}")))
    {
        return Err("只允许读取 Twitch 播放列表".into());
    }
    let http = common::client("https://player.twitch.tv/")?;
    let mut response = http
        .get(parsed)
        .header(reqwest::header::CACHE_CONTROL, "no-store")
        .send()
        .await
        .map_err(|_| "Twitch 播放列表连接失败")?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!("HTTP {}", status.as_u16()));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| "Twitch 播放列表读取失败")? {
        if bytes.len() + chunk.len() > MAX_PLAYLIST_BYTES {
            return Err("Twitch 播放列表过大".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    let text = String::from_utf8(bytes).map_err(|_| "Twitch 播放列表编码无效")?;
    if !text.starts_with("#EXTM3U") {
        return Err("Twitch 没有返回有效的播放列表".into());
    }
    Ok(text)
}

fn valid_login(login: &str) -> bool {
    const RESERVED: &[&str] = &["directory", "videos", "downloads", "settings", "search", "login", "p", "turbo", "jobs", "store"];
    (3..=25).contains(&login.len())
        && !RESERVED.contains(&login.to_ascii_lowercase().as_str())
        && login.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

fn rand_cache_buster() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as u64 % 900_000 + 100_000)
        .unwrap_or(123_456)
}

fn quality_key(variant: &Variant) -> u32 {
    variant.height * 1000 + variant.frame_rate
}

fn quality_label(variant: &Variant) -> String {
    if variant.frame_rate > 30 {
        format!("{}p{}", variant.height, variant.frame_rate)
    } else {
        format!("{}p", variant.height)
    }
}

/// Splits `KEY=VALUE,KEY="a,b"` attribute lists without breaking on quoted commas.
fn attributes(line: &str) -> Vec<(String, String)> {
    let mut pairs = Vec::new();
    let (mut key, mut value, mut in_key, mut quoted) = (String::new(), String::new(), true, false);
    for ch in line.chars() {
        match ch {
            '"' => quoted = !quoted,
            '=' if in_key && !quoted => in_key = false,
            ',' if !quoted => {
                pairs.push((std::mem::take(&mut key), std::mem::take(&mut value)));
                in_key = true;
            }
            _ if in_key => key.push(ch),
            _ => value.push(ch),
        }
    }
    if !key.is_empty() {
        pairs.push((key, value));
    }
    pairs
}

fn parse_master(text: &str) -> Result<Vec<Variant>, String> {
    let mut variants: Vec<Variant> = Vec::new();
    let mut lines = text.lines().map(str::trim);
    while let Some(line) = lines.next() {
        let Some(rest) = line.strip_prefix("#EXT-X-STREAM-INF:") else { continue };
        let attrs = attributes(rest);
        let get = |name: &str| attrs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str());
        let uri = lines.by_ref().find(|l| !l.is_empty() && !l.starts_with('#'));
        let (Some(resolution), Some(uri)) = (get("RESOLUTION"), uri) else { continue };
        if !get("CODECS").unwrap_or_default().contains("avc1") {
            continue;
        }
        let Some(height) = resolution.split('x').nth(1).and_then(|h| h.parse::<u32>().ok()) else {
            continue;
        };
        let frame_rate = get("FRAME-RATE")
            .and_then(|f| f.parse::<f64>().ok())
            .map(|f| f.round() as u32)
            .unwrap_or(30);
        let url = common::media_url(uri, PLAYLIST_DOMAINS).ok_or("Twitch 返回了不受信任的播放地址")?;
        variants.push(Variant { url, height, frame_rate });
    }
    variants.sort_by(|a, b| (b.height, b.frame_rate).cmp(&(a.height, a.frame_rate)));
    variants.dedup_by(|a, b| a.height == b.height && a.frame_rate == b.frame_rate);
    if variants.is_empty() {
        return Err("Twitch 没有可用的 H.264 画质".into());
    }
    Ok(variants)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MASTER: &str = r#"#EXTM3U
#EXT-X-SESSION-DATA:DATA-ID="NODE",VALUE="x"
#EXT-X-STREAM-INF:BANDWIDTH=230000,RESOLUTION=284x160,CODECS="avc1.4D401F,mp4a.40.2",FRAME-RATE=30.000
https://apn14.playlist.ttvnw.net/v1/playlist/a.m3u8
#EXT-X-STREAM-INF:BANDWIDTH=160000,CODECS="mp4a.40.2",STABLE-VARIANT-ID="audio_only"
https://apn14.playlist.ttvnw.net/v1/playlist/audio.m3u8
#EXT-X-STREAM-INF:BANDWIDTH=9147747,RESOLUTION=1920x1080,CODECS="avc1.64002A,mp4a.40.2",FRAME-RATE=60.000
https://apn14.playlist.ttvnw.net/v1/playlist/b.m3u8
#EXT-X-STREAM-INF:BANDWIDTH=3422999,RESOLUTION=1280x720,CODECS="avc1.4D401F,mp4a.40.2",FRAME-RATE=60.000
https://apn14.playlist.ttvnw.net/v1/playlist/c.m3u8
"#;

    #[test]
    fn parses_variants_best_first_and_skips_audio_only() {
        let variants = parse_master(MASTER).unwrap();
        let labels: Vec<_> = variants.iter().map(quality_label).collect();
        assert_eq!(labels, ["1080p60", "720p60", "160p"]);
        assert_eq!(quality_key(&variants[0]), 1_080_060);
    }

    #[test]
    fn rejects_untrusted_playlist_hosts() {
        let hostile = MASTER.replace("apn14.playlist.ttvnw.net", "evil.example.com");
        assert!(parse_master(&hostile).is_err());
    }

    #[test]
    fn quoted_commas_do_not_split_attributes() {
        let attrs = attributes(r#"BANDWIDTH=1,CODECS="avc1.4D,mp4a.40.2",FRAME-RATE=30"#);
        assert_eq!(attrs[1], ("CODECS".into(), "avc1.4D,mp4a.40.2".into()));
        assert_eq!(attrs.len(), 3);
    }

    #[test]
    fn channel_names_are_validated() {
        assert_eq!(TwitchClient::channel("https://www.twitch.tv/Jankos").unwrap(), "jankos");
        assert!(TwitchClient::channel("https://twitch.tv.evil.com/jankos").is_err());
        assert!(TwitchClient::channel("https://twitch.tv/directory").is_err());
        assert!(TwitchClient::channel("https://twitch.tv/a").is_err());
    }

    #[tokio::test]
    async fn playlist_proxy_only_allows_twitch_hosts() {
        assert!(fetch_playlist("https://example.com/a.m3u8").await.is_err());
        assert!(fetch_playlist("http://x.playlist.ttvnw.net/a.m3u8").await.is_err());
        assert!(fetch_playlist("https://x.playlist.ttvnw.net.evil.com/a.m3u8").await.is_err());
    }

    #[tokio::test]
    #[ignore = "network probe; set QILIU_TWITCH_PROBE to a live channel"]
    async fn resolves_live_channel() {
        let channel = std::env::var("QILIU_TWITCH_PROBE").unwrap_or_else(|_| "jankos".into());
        let client = TwitchClient::new().unwrap();
        let stream = client.resolve(&format!("https://www.twitch.tv/{channel}"), 0, 0).await.unwrap();
        assert!(stream.is_live, "channel offline");
        let url = stream.url.clone().unwrap();
        println!("qualities: {:?}", stream.quality_options);
        let playlist = fetch_playlist(&url).await.unwrap();
        assert!(playlist.contains("#EXTINF"));
    }
}
