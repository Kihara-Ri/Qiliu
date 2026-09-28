//! Douyu metadata and signed H5 playback, adapted from DouyinLiveRecorder.
use crate::{
    live_source::{self as common, Candidate},
    stream::{LiveStream, StreamLineOption, StreamQualityOption},
};
use boa_engine::{Context, Source};
use reqwest::Client;
use serde_json::Value;
use std::time::{SystemTime, UNIX_EPOCH};

const HOSTS: &[&str] = &["douyu.com", "www.douyu.com", "m.douyu.com"];
const CDN: &[&str] = &[
    "douyucdn.cn",
    "douyucdn2.cn",
    "douyucdn3.cn",
    "douyucdn4.cn",
    "douyucdn5.cn",
    "douyucdn6.cn",
    "douyu.com",
];
pub struct DouyuClient {
    http: Client,
}
impl DouyuClient {
    pub fn new() -> Result<Self, String> {
        Ok(Self {
            http: common::client("https://www.douyu.com/")?,
        })
    }
    async fn metadata(&self, source: &str) -> Result<(String, Value), String> {
        let url = common::source_url(source, HOSTS)?;
        let mut id = url
            .query_pairs()
            .find(|(k, _)| k == "rid")
            .map(|(_, v)| v.into_owned())
            .unwrap_or(common::room_slug(&url)?);
        if id.len() > 64
            || id.is_empty()
            || !id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        {
            return Err("无效的斗鱼房间号".into());
        }
        if !id.bytes().all(|b| b.is_ascii_digit()) {
            let html = common::body(
                self.http
                    .get(format!("https://m.douyu.com/{id}"))
                    .send()
                    .await,
            )
            .await?;
            let marker = "<script id=\"vike_pageContext\" type=\"application/json\">";
            let context = common::json_after(&html, marker)
                .ok_or("无法识别斗鱼房间别名，请使用数字房间链接")?;
            id = common::text(&context["pageProps"]["room"]["roomInfo"]["roomInfo"]["rid"]);
            if id.is_empty() || !id.bytes().all(|b| b.is_ascii_digit()) {
                return Err("无法识别斗鱼真实房间号".into());
            }
        }
        let data = common::json(
            self.http
                .get(format!("https://www.douyu.com/betard/{id}"))
                .send()
                .await,
        )
        .await;
        if let Ok(data) = data {
            if data["room"].get("show_status").is_some() {
                return Ok((id, data["room"].clone()));
            }
        }
        let data = common::json(
            self.http
                .get(format!("https://open.douyucdn.cn/api/RoomApi/room/{id}"))
                .send()
                .await,
        )
        .await?;
        if common::number(&data["error"]) != Some(0) {
            return Err("斗鱼房间不存在或暂时无法读取".into());
        }
        let mut room = data["data"].clone();
        room["show_status"] = room["room_status"].clone();
        // Open API does not expose videoLoop; use its authoritative room_status.
        Ok((id, room))
    }
    pub async fn is_live(&self, source: &str) -> Result<bool, String> {
        let (_, data) = self.metadata(source).await?;
        live_status(&data)
    }
    async fn play(&self, id: &str, signature: &str, rate: u32, cdn: &str) -> Result<Value, String> {
        let mut params: Vec<(String, String)> = url::form_urlencoded::parse(signature.as_bytes())
            .into_owned()
            .collect();
        params.extend([
            ("ver".into(), "22011191".into()),
            ("rid".into(), id.into()),
            ("rate".into(), rate.to_string()),
            ("cdn".into(), cdn.into()),
            ("iar".into(), "0".into()),
            ("ive".into(), "0".into()),
        ]);
        let response = common::json(
            self.http
                .post(format!("https://www.douyu.com/lapi/live/getH5Play/{id}"))
                .form(&params)
                .send()
                .await,
        )
        .await?;
        if common::number(&response["error"]) != Some(0) {
            return Err("斗鱼播放地址请求失败，可能需要验证或稍后重试".into());
        }
        Ok(response["data"].clone())
    }
    pub async fn resolve(
        &self,
        source: &str,
        line: usize,
        quality: u32,
    ) -> Result<LiveStream, String> {
        let (id, data) = self.metadata(source).await?;
        let mut stream = common::room(
            "douyu",
            "斗鱼",
            id.clone(),
            format!("https://www.douyu.com/{id}"),
        );
        stream.title = common::text(&data["room_name"]);
        stream.anchor = common::text(&data["nickname"]);
        stream.avatar_url = common::text(&data["avatar"]);
        if stream.avatar_url.is_empty() {
            stream.avatar_url = common::text(&data["avatar_mid"]);
        }
        stream.cover_url = common::text(&data["room_pic"]);
        stream.is_live = live_status(&data)?;
        if !stream.is_live {
            return Ok(stream);
        }
        let scripts = common::json(
            self.http
                .get("https://www.douyu.com/swf_api/homeH5Enc")
                .query(&[("rids", &id)])
                .send()
                .await,
        )
        .await?;
        let script = scripts["data"][format!("room{id}")]
            .as_str()
            .ok_or("斗鱼未返回签名脚本")?
            .to_owned();
        let signing_id = id.clone();
        let signature =
            tauri::async_runtime::spawn_blocking(move || sign(&script, &signing_id, timestamp()))
                .await
                .map_err(|_| "斗鱼签名任务失败")??;
        let mut play = self.play(&id, &signature, quality, "").await?;
        let cdns = play["cdnsWithName"].as_array().cloned().unwrap_or_default();
        let mut line_index = if cdns.is_empty() {
            0
        } else {
            line % cdns.len()
        };
        if let Some(cdn) = cdns.get(line_index).and_then(|v| v["cdn"].as_str()) {
            if play["cdn"].as_str() != Some(cdn) {
                play = self.play(&id, &signature, quality, cdn).await?;
            }
        }
        if let Some(actual) = play["cdn"].as_str() {
            if let Some(index) = cdns
                .iter()
                .position(|cdn| cdn["cdn"].as_str() == Some(actual))
            {
                line_index = index;
            }
        }
        apply_play(&mut stream, &play)?;
        if !cdns.is_empty() {
            stream.line_index = line_index;
            stream.line_count = cdns.len();
            stream.line_options = cdns
                .iter()
                .enumerate()
                .map(|(index, cdn)| StreamLineOption {
                    index,
                    label: cdn["name"].as_str().unwrap_or("斗鱼线路").into(),
                })
                .collect();
            stream.line_name = stream.line_options[line_index].label.clone();
        }
        Ok(stream)
    }
}
fn timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn live_status(room: &Value) -> Result<bool, String> {
    match common::number(&room["show_status"]) {
        Some(1) => Ok(common::number(&room["videoLoop"]).unwrap_or(0) == 0),
        Some(2 | 0) => Ok(false),
        _ => Err("斗鱼房间直播状态未知".into()),
    }
}
fn apply_play(stream: &mut LiveStream, play: &Value) -> Result<(), String> {
    let raw = format!(
        "{}/{}",
        common::text(&play["rtmp_url"]).trim_end_matches('/'),
        common::text(&play["rtmp_live"]).trim_start_matches('/')
    );
    let url = common::media_url(&raw, CDN).ok_or("斗鱼未返回受支持的直播线路")?;
    let rate = common::number(&play["rate"]).unwrap_or(0) as u32;
    let options: Vec<_> = play["multirates"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|q| {
            Some(StreamQualityOption {
                value: common::number(&q["rate"])? as u32,
                label: common::text(&q["name"]),
            })
        })
        .collect();
    let label = options
        .iter()
        .find(|q| q.value == rate)
        .map(|q| q.label.clone())
        .unwrap_or_else(|| "默认画质".into());
    let format = if url::Url::parse(&url)
        .ok()
        .is_some_and(|u| u.path().ends_with(".m3u8"))
    {
        "hls"
    } else {
        "flv"
    };
    common::select(
        stream,
        &[Candidate {
            url,
            format: format.into(),
            quality: rate,
            label,
        }],
        0,
        rate,
    )?;
    if !options.is_empty() {
        stream.quality_options = options;
    }
    Ok(())
}

fn evaluate(script: &str) -> Result<String, String> {
    if script.len() > 256 * 1024 {
        return Err("斗鱼签名脚本过大".into());
    }
    // No filesystem, network, process or frontend bindings are exposed.
    let mut context = Context::default();
    context
        .runtime_limits_mut()
        .set_loop_iteration_limit(100_000);
    context.runtime_limits_mut().set_recursion_limit(128);
    let value = context
        .eval(Source::from_bytes(script))
        .map_err(|_| "斗鱼签名脚本执行失败")?;
    value
        .as_string()
        .map(|s| s.to_std_string_escaped())
        .ok_or("斗鱼签名结果无效".into())
}
fn sign(script: &str, id: &str, time: u64) -> Result<String, String> {
    let did = format!("{:x}", md5::compute(format!("qiliu:{id}:{time}")));
    let eval = regex::Regex::new(r"eval[^;]*;\s*}").unwrap();
    let unpack = eval.replace_all(script, "strc;}");
    let decoded = evaluate(&format!("{unpack}\nub98484234();"))?;
    let version = regex::Regex::new(r"v=(\d+)").unwrap();
    let version = version
        .captures(&decoded)
        .and_then(|c| c.get(1))
        .ok_or("无法识别斗鱼签名版本")?
        .as_str();
    let digest = format!("{:x}", md5::compute(format!("{id}{did}{time}{version}")));
    let signing = regex::Regex::new(r"return rt;}\);?")
        .unwrap()
        .replace(&decoded, "return rt;}")
        .replace("(function (", "function sign(")
        .replace("CryptoJS.MD5(cb).toString()", &format!("\"{digest}\""));
    let params = evaluate(&format!(
        "{signing}\nsign({}, {}, {});",
        serde_json::to_string(id).unwrap(),
        serde_json::to_string(&did).unwrap(),
        serde_json::to_string(&time.to_string()).unwrap()
    ))?;
    let fields: Vec<_> = url::form_urlencoded::parse(params.as_bytes()).collect();
    if !["v", "did", "tt", "sign"]
        .iter()
        .all(|name| fields.iter().any(|(k, v)| k == name && !v.is_empty()))
    {
        return Err("斗鱼签名缺少必要字段".into());
    }
    Ok(params)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn recognizes_live_offline_and_replay() {
        assert!(live_status(&json!({"show_status":1,"videoLoop":0})).unwrap());
        assert!(!live_status(&json!({"show_status":1,"videoLoop":1})).unwrap());
        assert!(!live_status(&json!({"show_status":"2"})).unwrap());
        assert!(live_status(&json!({})).is_err());
    }
    #[test]
    fn parses_actual_quality_and_stream_transport() {
        let mut stream = common::room("douyu", "斗鱼", "1".into(), "".into());
        apply_play(&mut stream,&json!({"rtmp_url":"http://pull.douyucdn.cn/live", "rtmp_live":"room.flv?token=test", "rate":3, "multirates":[{"rate":0,"name":"原画"},{"rate":3,"name":"超清"}]})).unwrap();
        assert_eq!(stream.quality_label, "超清");
        assert_eq!(stream.quality_options.len(), 2);
        assert!(stream.url.unwrap().starts_with("https://"));
    }
    #[test]
    fn javascript_has_no_host_access_and_is_bounded() {
        assert!(evaluate("process.env").is_err());
        assert!(evaluate("while(true) {}").is_err());
        assert_eq!(evaluate("'ok'").unwrap(), "ok");
    }
    #[test]
    fn signs_unpacked_platform_function() {
        let unpacked = "(function (rid,did,tt){var v='v=123';var cb='';var rt=v+'&did='+did+'&tt='+tt+'&sign='+CryptoJS.MD5(cb).toString();return rt;});";
        let script = format!(
            "function ub98484234(){{var strc={};return eval(strc);}}",
            serde_json::to_string(unpacked).unwrap()
        );
        let params = sign(&script, "9999", 1234567890).unwrap();
        assert!(params.contains("tt=1234567890"));
        assert!(params.contains("sign="));
    }
}
