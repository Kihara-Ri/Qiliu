//! CCTV channel live sources through the CNTV web-player CDN.
//!
//! tv.cctv.com's player (js.player.cntv.cn/creator/liveplayer.js) ships an
//! anonymous multi-rate HLS fallback. The CDN master playlists are static
//! templates: advertised variants only exist for channels the web player
//! guarantees without mainland entitlement (today CCTV-1 and CCTV-13; the
//! runtime vdn API that serves the rest answers non-entitled clients with
//! obfuscated URLs). The channel table therefore only lists channels whose
//! variants have been verified, and resolve() additionally checks each
//! advertised variant before offering it.
use crate::{
    live_source::{self as common},
    stream::{LiveStream, StreamLineOption, StreamQualityOption},
};
use reqwest::Client;

const HOSTS: &[&str] = &["tv.cctv.com", "www.cctv.com", "live.cctv.com", "cctv.com"];
const CDN_HOST: &str = "ldncctvwbcdtxy.liveplay.myqcloud.com";
const CDN_PATH: &str = "/ldncctvwbcd/";

const CHANNELS: &[(&str, &str)] = &[
    ("cctv1", "CCTV-1 综合"),
    ("cctv13", "CCTV-13 新闻"),
];

pub struct CctvClient {
    http: Client,
}

impl CctvClient {
    pub fn new() -> Result<Self, String> {
        Ok(Self {
            http: common::client("https://tv.cctv.com/")?,
        })
    }

    pub async fn is_live(&self, source: &str) -> Result<bool, String> {
        let slug = channel_slug(source)?;
        Ok(!self.usable_variants(&slug).await?.is_empty())
    }

    pub async fn resolve(
        &self,
        source: &str,
        _line: usize,
        quality: u32,
    ) -> Result<LiveStream, String> {
        let slug = channel_slug(source)?;
        let (label, _) = CHANNELS
            .iter()
            .find(|(id, _)| *id == slug)
            .ok_or_else(unknown_channel)?;
        let variants = self.usable_variants(&slug).await?;
        let mut stream = common::room("cctv", "央视", slug.clone(), canonical(&slug));
        stream.title = (*label).into();
        stream.anchor = (*label).into();
        stream.is_live = true;
        stream.line_index = 0;
        stream.line_count = 1;
        stream.line_name = "线路 1 · 腾讯云 CDN".into();
        stream.line_options = vec![StreamLineOption {
            index: 0,
            label: stream.line_name.clone(),
        }];
        for variant in &variants {
            stream.quality_options.push(StreamQualityOption {
                value: variant.quality,
                label: variant.label.clone(),
            });
        }
        let chosen = variants
            .iter()
            .find(|variant| variant.quality == quality)
            .unwrap_or(&variants[0]);
        stream.url = Some(variant_url(&slug, &chosen.path)?);
        stream.format = "hls".into();
        stream.quality_label = chosen.label.clone();
        stream.bitrate = chosen.bandwidth_kbps;
        Ok(stream)
    }

    /// Return the master's variants that actually answer with a playlist;
    /// the template advertises rates the CDN may not carry.
    async fn usable_variants(&self, slug: &str) -> Result<Vec<Variant>, String> {
        let master = self
            .playlist(master_url(slug).as_str())
            .await?
            .ok_or("央视频道当前不可用，可能处于版权保护时段或已停播")?;
        let variants = parse_master(&master);
        let mut usable = Vec::new();
        for variant in variants {
            let url = variant_url(slug, &variant.path)?;
            // Variants that answer 403/404 are dropped from the menu; a
            // transport failure propagates as a connection error instead.
            if matches!(self.playlist(&url).await, Ok(Some(_))) {
                usable.push(variant);
            }
        }
        if usable.is_empty() {
            return Err("央视频道没有可用的直播画质，请稍后重试".into());
        }
        Ok(usable)
    }

    async fn playlist(&self, url: &str) -> Result<Option<String>, String> {
        let response = match self.http.get(url).send().await {
            Ok(response) => response,
            Err(_) => return Err("央视频道连接失败，请稍后重试".into()),
        };
        if !response.status().is_success() {
            // Copyright windows can answer 403.
            return Ok(None);
        }
        let text = common::body(Ok(response)).await?;
        if !text.starts_with("#EXTM3U") {
            return Err("央视频道返回了无效的播放列表".into());
        }
        Ok(Some(text))
    }
}

fn unknown_channel() -> String {
    "该央视频道暂不受支持；目前支持 CCTV-1（https://tv.cctv.com/live/cctv1/）和 CCTV-13（https://tv.cctv.com/live/cctv13/）".into()
}

fn canonical(slug: &str) -> String {
    format!("https://tv.cctv.com/live/{slug}/")
}

fn master_url(slug: &str) -> url::Url {
    url::Url::parse(&format!("https://{CDN_HOST}{CDN_PATH}cdrmld{slug}_1/index.m3u8"))
        .expect("CDN master URL is built from constants")
}

fn variant_url(slug: &str, path: &str) -> Result<String, String> {
    let url = master_url(slug)
        .join(path)
        .map_err(|_| "央视频道播放地址无效".to_string())?;
    // Variant lines come from the CDN response; keep them on the same host so
    // a rewritten playlist cannot point playback elsewhere.
    if url.scheme() != "https"
        || url.host_str() != Some(CDN_HOST)
        || url.username() != ""
        || url.password().is_some()
        || url.port().is_some()
        || !url.path().starts_with(CDN_PATH)
    {
        return Err("央视频道播放线路异常".into());
    }
    Ok(url.into())
}

fn channel_slug(source: &str) -> Result<String, String> {
    let url = common::source_url(source, HOSTS)?;
    let mut segments = url
        .path_segments()
        .into_iter()
        .flatten()
        .filter(|s| !s.is_empty());
    if segments.next() != Some("live") {
        return Err(unknown_channel());
    }
    let slug = segments.next().unwrap_or_default().to_ascii_lowercase();
    if !CHANNELS.iter().any(|(id, _)| *id == slug) {
        return Err(unknown_channel());
    }
    Ok(slug)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Variant {
    quality: u32,
    label: String,
    path: String,
    bandwidth_kbps: u32,
}

fn parse_master(text: &str) -> Vec<Variant> {
    let mut variants: Vec<Variant> = Vec::new();
    let mut pending: Option<(u32, u32)> = None; // (height, bandwidth_kbps)
    for line in text.lines() {
        let line = line.trim();
        if let Some(attributes) = line.strip_prefix("#EXT-X-STREAM-INF:") {
            let height = regex_attr(attributes, "RESOLUTION")
                .and_then(|value| value.rsplit('x').next().map(str::to_owned))
                .and_then(|value| value.parse().ok());
            let bandwidth_kbps = regex_attr(attributes, "BANDWIDTH")
                .and_then(|value| value.parse::<u32>().ok())
                .map(|bps| bps / 1000);
            pending = height.zip(bandwidth_kbps);
            continue;
        }
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((height, bandwidth_kbps)) = pending.take() {
            let label = format!("{height}P");
            if !variants.iter().any(|variant| variant.quality == height) {
                variants.push(Variant {
                    quality: height,
                    label,
                    path: line.to_owned(),
                    bandwidth_kbps,
                });
            }
        }
    }
    variants.sort_by(|a, b| b.quality.cmp(&a.quality));
    if variants.is_empty() && text.contains("#EXTINF") {
        // A channel may answer with a plain media playlist instead of a master.
        variants.push(Variant {
            quality: 0,
            label: "自动".into(),
            path: String::new(),
            bandwidth_kbps: 0,
        });
    }
    variants
}

fn regex_attr(attributes: &str, name: &str) -> Option<String> {
    let pattern = regex::Regex::new(&format!(r#"(?:^|,){name}=([^,]+)"#)).ok()?;
    pattern
        .captures(attributes)
        .map(|captures| captures[1].trim().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_channel_slug_from_official_links() {
        assert_eq!(
            channel_slug("https://tv.cctv.com/live/cctv1/").unwrap(),
            "cctv1"
        );
        assert_eq!(
            channel_slug("https://tv.cctv.com/live/cctv13/index.shtml").unwrap(),
            "cctv13"
        );
        assert_eq!(
            channel_slug("https://www.cctv.com/live/cctv1/").unwrap(),
            "cctv1"
        );
        // Templates for other channels exist on the CDN but carry no streams.
        assert!(channel_slug("https://tv.cctv.com/live/cctv5/").is_err());
        assert!(channel_slug("https://tv.cctv.com/live/cctvjilu/").is_err());
        assert!(channel_slug("https://tv.cctv.com/lm/cctv1/").is_err());
        assert!(channel_slug("https://tv.cctv.com/2026/01/01/VIDEO.shtml").is_err());
        assert!(channel_slug("https://example.com/live/cctv1/").is_err());
    }

    #[test]
    fn orders_master_variants_from_highest_quality() {
        let master = "#EXTM3U\n\
            #EXT-X-VERSION:3\n\
            #EXT-X-STREAM-INF:BANDWIDTH=3200000,RESOLUTION=1920x1080\n\
            /ldncctvwbcd/cdrmldcctv1_1_pd.m3u8\n\
            #EXT-X-STREAM-INF:BANDWIDTH=900000,RESOLUTION=854x480\n\
            /ldncctvwbcd/cdrmldcctv1_1_hd.m3u8\n\
            #EXT-X-STREAM-INF:BANDWIDTH=1800000,RESOLUTION=1280x720\n\
            /ldncctvwbcd/cdrmldcctv1_1_td.m3u8\n";
        let variants = parse_master(master);
        let qualities: Vec<u32> = variants.iter().map(|v| v.quality).collect();
        assert_eq!(qualities, [1080, 720, 480]);
        assert_eq!(variants[0].label, "1080P");
        assert_eq!(variants[0].bandwidth_kbps, 3200);
        assert_eq!(variants[0].path, "/ldncctvwbcd/cdrmldcctv1_1_pd.m3u8");
    }

    #[test]
    fn keeps_single_variant_and_media_playlist_fallbacks() {
        let single = "#EXTM3U\n\
            #EXT-X-STREAM-INF:BANDWIDTH=800000,RESOLUTION=960x540\n\
            /ldncctvwbcd/cdrmldcctv1_1_hd.m3u8\n";
        assert_eq!(parse_master(single).len(), 1);
        let media = "#EXTM3U\n#EXT-X-TARGETDURATION:4\n#EXTINF:4.000,\nseg-1.ts\n";
        let variants = parse_master(media);
        assert_eq!(variants.len(), 1);
        assert_eq!(variants[0].label, "自动");
        assert!(parse_master("#EXTM3U\n").is_empty());
    }

    #[test]
    fn builds_variant_urls_on_the_cdn_host_only() {
        assert_eq!(
            variant_url("cctv1", "/ldncctvwbcd/cdrmldcctv1_1_pd.m3u8").unwrap(),
            "https://ldncctvwbcdtxy.liveplay.myqcloud.com/ldncctvwbcd/cdrmldcctv1_1_pd.m3u8"
        );
        // Relative references resolve against the master playlist's directory.
        assert_eq!(
            variant_url("cctv1", "cdrmldcctv1_1_pd.m3u8").unwrap(),
            "https://ldncctvwbcdtxy.liveplay.myqcloud.com/ldncctvwbcd/cdrmldcctv1_1/cdrmldcctv1_1_pd.m3u8"
        );
        assert!(variant_url("cctv1", "https://evil.test/stream.m3u8").is_err());
        assert!(variant_url("cctv1", "/other/path.m3u8").is_err());
    }
}
