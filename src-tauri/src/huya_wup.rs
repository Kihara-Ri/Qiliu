use reqwest::{
    header::{HeaderMap, HeaderValue, CONTENT_TYPE, ORIGIN, REFERER, USER_AGENT},
    Client,
};
use url::Url;

const HUYA_WUP_ENDPOINT: &str = "https://wup.huya.com/";
const HUYA_MOBILE_ORIGIN: &str = "https://m.huya.com/";
const HUYA_SDK_USER_AGENT: &str =
    "HYSDK(Windows, 30000002)_APP(pc_exe&7060000&official)_SDK(trans&2.32.3.5646)";
const MAX_WUP_RESPONSE_BYTES: usize = 8 * 1024 * 1024;

const TYPE_BYTE: u8 = 0;
const TYPE_SHORT: u8 = 1;
const TYPE_INT: u8 = 2;
const TYPE_LONG: u8 = 3;
const TYPE_FLOAT: u8 = 4;
const TYPE_DOUBLE: u8 = 5;
const TYPE_STRING1: u8 = 6;
const TYPE_STRING4: u8 = 7;
const TYPE_MAP: u8 = 8;
const TYPE_LIST: u8 = 9;
const TYPE_STRUCT_BEGIN: u8 = 10;
const TYPE_STRUCT_END: u8 = 11;
const TYPE_ZERO: u8 = 12;
const TYPE_SIMPLE_LIST: u8 = 13;

#[derive(Debug, Clone, PartialEq)]
pub struct ReplaySource {
    pub url: String,
    pub sync_time_seconds: f64,
    pub quality_label: String,
    pub nominal_bitrate_kbps: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ReplayDefinition {
    width: u32,
    height: u32,
    definition: String,
    m3u8: String,
    name: String,
    codec: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ReplayHistory {
    hls_url: String,
    sync_time_seconds: u64,
    definitions: Vec<ReplayDefinition>,
}

pub async fn resolve_replay(http: &Client, presenter_uid: u64) -> Result<ReplaySource, String> {
    if presenter_uid == 0 || presenter_uid > i64::MAX as u64 {
        return Err("虎牙回放缺少有效的主播标识".to_string());
    }

    let body = build_replay_request(presenter_uid as i64);
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/x-wup"));
    headers.insert(ORIGIN, HeaderValue::from_static(HUYA_MOBILE_ORIGIN));
    headers.insert(REFERER, HeaderValue::from_static(HUYA_MOBILE_ORIGIN));
    headers.insert(USER_AGENT, HeaderValue::from_static(HUYA_SDK_USER_AGENT));

    let response = http
        .post(HUYA_WUP_ENDPOINT)
        .headers(headers)
        .body(body)
        .send()
        .await
        .map_err(|error| format!("连接虎牙回放接口失败：{error}"))?;
    if !response.status().is_success() {
        return Err(format!("虎牙回放接口返回状态 {}", response.status()));
    }

    let bytes = response
        .bytes()
        .await
        .map_err(|error| format!("读取虎牙回放信息失败：{error}"))?;
    if bytes.len() > MAX_WUP_RESPONSE_BYTES {
        return Err("虎牙回放信息超出允许大小".to_string());
    }

    let histories = parse_replay_response(&bytes)?;
    let history = histories
        .into_iter()
        .next()
        .ok_or_else(|| "虎牙当前没有可用的回放录像".to_string())?;
    select_replay_source(history)
}

fn select_replay_source(history: ReplayHistory) -> Result<ReplaySource, String> {
    let selected = history
        .definitions
        .iter()
        .filter(|definition| !definition.m3u8.trim().is_empty())
        .max_by_key(|definition| {
            let h264_preference = u8::from(
                definition.codec.is_empty() || definition.codec.eq_ignore_ascii_case("h264"),
            );
            let pixels = u64::from(definition.width) * u64::from(definition.height);
            let nominal = definition.definition.parse::<u32>().unwrap_or_default();
            (h264_preference, pixels, nominal)
        });

    let (raw_url, quality_label, nominal_bitrate_kbps) = if let Some(definition) = selected {
        let resolution = if definition.height > 0 {
            format!("{}P", definition.height)
        } else {
            String::new()
        };
        let quality_label = match (resolution.is_empty(), definition.name.trim().is_empty()) {
            (false, false) => format!("{resolution} · {}", definition.name.trim()),
            (false, true) => resolution,
            (true, false) => definition.name.trim().to_string(),
            (true, true) => "回放".to_string(),
        };
        (
            definition.m3u8.as_str(),
            quality_label,
            definition.definition.parse::<u32>().unwrap_or_default(),
        )
    } else {
        (history.hls_url.as_str(), "回放".to_string(), 0)
    };

    Ok(ReplaySource {
        url: secure_huya_media_url(raw_url)?,
        sync_time_seconds: history.sync_time_seconds as f64,
        quality_label,
        nominal_bitrate_kbps,
    })
}

fn secure_huya_media_url(raw: &str) -> Result<String, String> {
    let mut url = Url::parse(raw.trim()).map_err(|_| "虎牙回放地址格式不正确".to_string())?;
    if url.scheme() == "http" {
        url.set_scheme("https")
            .map_err(|_| "虎牙回放地址无法升级为 HTTPS".to_string())?;
    }
    if url.scheme() != "https" {
        return Err("虎牙回放只允许 HTTPS 地址".to_string());
    }
    let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
    if host != "huya.com" && !host.ends_with(".huya.com") {
        return Err("虎牙回放返回了非虎牙媒体地址".to_string());
    }
    Ok(url.to_string())
}

fn build_replay_request(presenter_uid: i64) -> Vec<u8> {
    let mut t_req = TarsWriter::default();
    t_req.write_struct(0, |writer| {
        writer.write_integer(presenter_uid, 0);
        writer.write_integer(0, 1);
        writer.write_struct(2, |writer| {
            writer.write_integer(0, 0);
            writer.write_string("", 1);
            writer.write_string("", 2);
            writer.write_string("", 3);
            writer.write_string("", 4);
            writer.write_integer(0, 5);
        });
    });

    let mut params = TarsWriter::default();
    params.write_string_bytes_map("tReq", &t_req.bytes, 0);

    let mut packet = TarsWriter::default();
    packet.write_integer(3, 1);
    packet.write_integer(0, 2);
    packet.write_integer(0, 3);
    packet.write_integer(0, 4);
    packet.write_string("liveui", 5);
    packet.write_string("getVideoHisUpon", 6);
    packet.write_bytes(&params.bytes, 7);
    packet.write_integer(0, 8);
    packet.write_empty_map(9);
    packet.write_empty_map(10);

    let packet_size = packet.bytes.len().saturating_add(4);
    let mut output = Vec::with_capacity(packet_size);
    output.extend_from_slice(&(packet_size as u32).to_be_bytes());
    output.extend_from_slice(&packet.bytes);
    output
}

fn parse_replay_response(bytes: &[u8]) -> Result<Vec<ReplayHistory>, String> {
    if bytes.len() < 4 {
        return Err("虎牙回放响应过短".to_string());
    }
    let declared_size = u32::from_be_bytes(bytes[..4].try_into().unwrap()) as usize;
    if declared_size < 4 || declared_size > bytes.len() {
        return Err("虎牙回放响应长度不正确".to_string());
    }

    let mut packet = TarsReader::new(&bytes[4..declared_size]);
    let params = packet
        .read_bytes(7)?
        .ok_or_else(|| "虎牙回放响应缺少参数数据".to_string())?;
    let mut params_reader = TarsReader::new(params);
    let response = params_reader
        .read_string_bytes_map_value(0, "tRsp")?
        .ok_or_else(|| "虎牙回放响应缺少 tRsp".to_string())?;

    let mut response_reader = TarsReader::new(response);
    response_reader.read_struct_begin(0)?;
    let _ = response_reader.read_integer(0)?;
    let history_count = response_reader.read_list_len(1)?.unwrap_or_default();
    let mut histories = Vec::with_capacity(history_count.min(16));
    for _ in 0..history_count {
        response_reader.read_struct_begin(0)?;
        let hls_url = response_reader.read_string(1)?.unwrap_or_default();
        let sync_time_seconds = response_reader.read_integer(2)?.unwrap_or_default().max(0) as u64;
        let definition_count = response_reader.read_list_len(3)?.unwrap_or_default();
        let mut definitions = Vec::with_capacity(definition_count.min(16));
        for _ in 0..definition_count {
            response_reader.read_struct_begin(0)?;
            let _ = response_reader.read_string(0)?;
            let width = parse_u32(response_reader.read_string(1)?);
            let height = parse_u32(response_reader.read_string(2)?);
            let definition = response_reader.read_string(3)?.unwrap_or_default();
            let _ = response_reader.read_string(4)?;
            let m3u8 = response_reader.read_string(5)?.unwrap_or_default();
            let name = response_reader.read_string(6)?.unwrap_or_default();
            let codec = response_reader.read_string(13)?.unwrap_or_default();
            response_reader.finish_struct()?;
            definitions.push(ReplayDefinition {
                width,
                height,
                definition,
                m3u8,
                name,
                codec,
            });
        }
        response_reader.finish_struct()?;
        histories.push(ReplayHistory {
            hls_url,
            sync_time_seconds,
            definitions,
        });
    }
    response_reader.finish_struct()?;
    Ok(histories)
}

fn parse_u32(value: Option<String>) -> u32 {
    value
        .as_deref()
        .and_then(|value| value.parse().ok())
        .unwrap_or_default()
}

#[derive(Default)]
struct TarsWriter {
    bytes: Vec<u8>,
}

impl TarsWriter {
    fn write_head(&mut self, field_type: u8, tag: u8) {
        if tag < 15 {
            self.bytes.push((tag << 4) | field_type);
        } else {
            self.bytes.push((15 << 4) | field_type);
            self.bytes.push(tag);
        }
    }

    fn write_integer(&mut self, value: i64, tag: u8) {
        if value == 0 {
            self.write_head(TYPE_ZERO, tag);
        } else if let Ok(value) = i8::try_from(value) {
            self.write_head(TYPE_BYTE, tag);
            self.bytes.push(value as u8);
        } else if let Ok(value) = i16::try_from(value) {
            self.write_head(TYPE_SHORT, tag);
            self.bytes.extend_from_slice(&value.to_be_bytes());
        } else if let Ok(value) = i32::try_from(value) {
            self.write_head(TYPE_INT, tag);
            self.bytes.extend_from_slice(&value.to_be_bytes());
        } else {
            self.write_head(TYPE_LONG, tag);
            self.bytes.extend_from_slice(&value.to_be_bytes());
        }
    }

    fn write_string(&mut self, value: &str, tag: u8) {
        let bytes = value.as_bytes();
        if bytes.len() <= u8::MAX as usize {
            self.write_head(TYPE_STRING1, tag);
            self.bytes.push(bytes.len() as u8);
        } else {
            self.write_head(TYPE_STRING4, tag);
            self.bytes
                .extend_from_slice(&(bytes.len() as u32).to_be_bytes());
        }
        self.bytes.extend_from_slice(bytes);
    }

    fn write_bytes(&mut self, value: &[u8], tag: u8) {
        self.write_head(TYPE_SIMPLE_LIST, tag);
        self.write_head(TYPE_BYTE, 0);
        self.write_integer(value.len() as i64, 0);
        self.bytes.extend_from_slice(value);
    }

    fn write_empty_map(&mut self, tag: u8) {
        self.write_head(TYPE_MAP, tag);
        self.write_integer(0, 0);
    }

    fn write_string_bytes_map(&mut self, key: &str, value: &[u8], tag: u8) {
        self.write_head(TYPE_MAP, tag);
        self.write_integer(1, 0);
        self.write_string(key, 0);
        self.write_bytes(value, 1);
    }

    #[cfg(test)]
    fn write_list<F>(&mut self, tag: u8, len: usize, write_items: F)
    where
        F: FnOnce(&mut Self),
    {
        self.write_head(TYPE_LIST, tag);
        self.write_integer(len as i64, 0);
        write_items(self);
    }

    fn write_struct<F>(&mut self, tag: u8, write_fields: F)
    where
        F: FnOnce(&mut Self),
    {
        self.write_head(TYPE_STRUCT_BEGIN, tag);
        write_fields(self);
        self.write_head(TYPE_STRUCT_END, 0);
    }
}

#[derive(Debug, Clone, Copy)]
struct TarsHead {
    tag: u8,
    field_type: u8,
}

struct TarsReader<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> TarsReader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, position: 0 }
    }

    fn read_head(&mut self) -> Result<TarsHead, String> {
        let first = *self
            .bytes
            .get(self.position)
            .ok_or_else(|| "虎牙回放响应意外结束".to_string())?;
        self.position += 1;
        let field_type = first & 0x0f;
        let mut tag = first >> 4;
        if tag == 15 {
            tag = *self
                .bytes
                .get(self.position)
                .ok_or_else(|| "虎牙回放字段标签不完整".to_string())?;
            self.position += 1;
        }
        Ok(TarsHead { tag, field_type })
    }

    fn peek_head(&mut self) -> Result<TarsHead, String> {
        let position = self.position;
        let head = self.read_head()?;
        self.position = position;
        Ok(head)
    }

    fn skip_to_tag(&mut self, tag: u8) -> Result<bool, String> {
        while self.position < self.bytes.len() {
            let head = self.peek_head()?;
            if head.field_type == TYPE_STRUCT_END || head.tag >= tag {
                return Ok(head.tag == tag && head.field_type != TYPE_STRUCT_END);
            }
            let head = self.read_head()?;
            self.skip_value(head.field_type)?;
        }
        Ok(false)
    }

    fn skip_value(&mut self, field_type: u8) -> Result<(), String> {
        match field_type {
            TYPE_BYTE => self.advance(1),
            TYPE_SHORT => self.advance(2),
            TYPE_INT | TYPE_FLOAT => self.advance(4),
            TYPE_LONG | TYPE_DOUBLE => self.advance(8),
            TYPE_STRING1 => {
                let len = self.read_unsigned(1)? as usize;
                self.advance(len)
            }
            TYPE_STRING4 => {
                let len = self.read_unsigned(4)? as usize;
                self.advance(len)
            }
            TYPE_MAP => {
                let len = self.read_required_integer(0)?.max(0) as usize;
                for _ in 0..len.saturating_mul(2) {
                    let head = self.read_head()?;
                    self.skip_value(head.field_type)?;
                }
                Ok(())
            }
            TYPE_LIST => {
                let len = self.read_required_integer(0)?.max(0) as usize;
                for _ in 0..len {
                    let head = self.read_head()?;
                    self.skip_value(head.field_type)?;
                }
                Ok(())
            }
            TYPE_STRUCT_BEGIN => self.finish_struct(),
            TYPE_STRUCT_END | TYPE_ZERO => Ok(()),
            TYPE_SIMPLE_LIST => {
                let subtype = self.read_head()?;
                if subtype.field_type != TYPE_BYTE {
                    return Err("虎牙回放字节列表类型不正确".to_string());
                }
                let len = self.read_required_integer(0)?.max(0) as usize;
                self.advance(len)
            }
            _ => Err("虎牙回放包含未知字段类型".to_string()),
        }
    }

    fn read_integer(&mut self, tag: u8) -> Result<Option<i64>, String> {
        if !self.skip_to_tag(tag)? {
            return Ok(None);
        }
        let head = self.read_head()?;
        Ok(Some(self.read_integer_payload(head.field_type)?))
    }

    fn read_required_integer(&mut self, tag: u8) -> Result<i64, String> {
        self.read_integer(tag)?
            .ok_or_else(|| "虎牙回放响应缺少整数字段".to_string())
    }

    fn read_integer_payload(&mut self, field_type: u8) -> Result<i64, String> {
        match field_type {
            TYPE_ZERO => Ok(0),
            TYPE_BYTE => Ok(self.read_signed(1)?),
            TYPE_SHORT => Ok(self.read_signed(2)?),
            TYPE_INT => Ok(self.read_signed(4)?),
            TYPE_LONG => self.read_signed(8),
            _ => Err("虎牙回放整数字段类型不正确".to_string()),
        }
    }

    fn read_string(&mut self, tag: u8) -> Result<Option<String>, String> {
        if !self.skip_to_tag(tag)? {
            return Ok(None);
        }
        let head = self.read_head()?;
        let len = match head.field_type {
            TYPE_STRING1 => self.read_unsigned(1)? as usize,
            TYPE_STRING4 => self.read_unsigned(4)? as usize,
            _ => return Err("虎牙回放字符串字段类型不正确".to_string()),
        };
        let bytes = self.take(len)?;
        let value =
            std::str::from_utf8(bytes).map_err(|_| "虎牙回放字符串编码不正确".to_string())?;
        Ok(Some(value.to_string()))
    }

    fn read_bytes(&mut self, tag: u8) -> Result<Option<&'a [u8]>, String> {
        if !self.skip_to_tag(tag)? {
            return Ok(None);
        }
        let head = self.read_head()?;
        if head.field_type != TYPE_SIMPLE_LIST {
            return Err("虎牙回放字节字段类型不正确".to_string());
        }
        let subtype = self.read_head()?;
        if subtype.field_type != TYPE_BYTE {
            return Err("虎牙回放字节字段子类型不正确".to_string());
        }
        let len = self.read_required_integer(0)?.max(0) as usize;
        Ok(Some(self.take(len)?))
    }

    fn read_list_len(&mut self, tag: u8) -> Result<Option<usize>, String> {
        if !self.skip_to_tag(tag)? {
            return Ok(None);
        }
        let head = self.read_head()?;
        if head.field_type != TYPE_LIST {
            return Err("虎牙回放列表字段类型不正确".to_string());
        }
        let len = self.read_required_integer(0)?.max(0) as usize;
        if len > 10_000 {
            return Err("虎牙回放列表长度超出允许范围".to_string());
        }
        Ok(Some(len))
    }

    fn read_struct_begin(&mut self, tag: u8) -> Result<(), String> {
        if !self.skip_to_tag(tag)? {
            return Err("虎牙回放响应缺少结构字段".to_string());
        }
        let head = self.read_head()?;
        if head.field_type != TYPE_STRUCT_BEGIN {
            return Err("虎牙回放结构字段类型不正确".to_string());
        }
        Ok(())
    }

    fn finish_struct(&mut self) -> Result<(), String> {
        loop {
            let head = self.read_head()?;
            if head.field_type == TYPE_STRUCT_END {
                return Ok(());
            }
            self.skip_value(head.field_type)?;
        }
    }

    fn read_string_bytes_map_value(
        &mut self,
        tag: u8,
        wanted_key: &str,
    ) -> Result<Option<&'a [u8]>, String> {
        if !self.skip_to_tag(tag)? {
            return Ok(None);
        }
        let head = self.read_head()?;
        if head.field_type != TYPE_MAP {
            return Err("虎牙回放参数字段类型不正确".to_string());
        }
        let len = self.read_required_integer(0)?.max(0) as usize;
        if len > 128 {
            return Err("虎牙回放参数数量超出允许范围".to_string());
        }
        let mut found = None;
        for _ in 0..len {
            let key = self.read_string(0)?.unwrap_or_default();
            let value = self
                .read_bytes(1)?
                .ok_or_else(|| "虎牙回放参数缺少字节值".to_string())?;
            if key == wanted_key {
                found = Some(value);
            }
        }
        Ok(found)
    }

    fn read_signed(&mut self, len: usize) -> Result<i64, String> {
        let bytes = self.take(len)?;
        Ok(match len {
            1 => i8::from_be_bytes([bytes[0]]) as i64,
            2 => i16::from_be_bytes(bytes.try_into().unwrap()) as i64,
            4 => i32::from_be_bytes(bytes.try_into().unwrap()) as i64,
            8 => i64::from_be_bytes(bytes.try_into().unwrap()),
            _ => return Err("虎牙回放整数长度不正确".to_string()),
        })
    }

    fn read_unsigned(&mut self, len: usize) -> Result<u64, String> {
        let bytes = self.take(len)?;
        Ok(match len {
            1 => bytes[0] as u64,
            4 => u32::from_be_bytes(bytes.try_into().unwrap()) as u64,
            _ => return Err("虎牙回放长度字段不正确".to_string()),
        })
    }

    fn advance(&mut self, len: usize) -> Result<(), String> {
        let _ = self.take(len)?;
        Ok(())
    }

    fn take(&mut self, len: usize) -> Result<&'a [u8], String> {
        let end = self
            .position
            .checked_add(len)
            .filter(|end| *end <= self.bytes.len())
            .ok_or_else(|| "虎牙回放响应字段越界".to_string())?;
        let value = &self.bytes[self.position..end];
        self.position = end;
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replay_request_contains_expected_wup_target() {
        let request = build_replay_request(1_634_546_845);
        assert_eq!(
            u32::from_be_bytes(request[..4].try_into().unwrap()) as usize,
            request.len()
        );
        assert!(request.windows(6).any(|window| window == b"liveui"));
        assert!(request
            .windows(15)
            .any(|window| window == b"getVideoHisUpon"));
    }

    #[test]
    fn parses_response_and_selects_highest_h264_replay() {
        let response = synthetic_response();
        let histories = parse_replay_response(&response).unwrap();
        let source = select_replay_source(histories.into_iter().next().unwrap()).unwrap();

        assert_eq!(source.sync_time_seconds, 19_637.0);
        assert_eq!(source.quality_label, "1080P · 原画");
        assert_eq!(source.nominal_bitrate_kbps, 4_000);
        assert_eq!(
            source.url,
            "https://videoal-platform.cdn.huya.com/replay-1080.m3u8"
        );
    }

    #[test]
    fn rejects_non_huya_replay_urls() {
        assert!(secure_huya_media_url("https://example.com/replay.m3u8").is_err());
    }

    #[test]
    #[ignore = "requires the current public Huya replay service"]
    fn resolves_reference_room_replay_at_1080p() {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(20))
            .build()
            .unwrap();
        let source = tauri::async_runtime::block_on(resolve_replay(&client, 1_634_546_845))
            .expect("reference room should expose replay history");

        assert!(source.quality_label.starts_with("1080P"));
        assert!(source.sync_time_seconds > 0.0);
        assert!(source.url.contains(".huya.com/"));
    }

    fn synthetic_response() -> Vec<u8> {
        let mut t_rsp = TarsWriter::default();
        t_rsp.write_struct(0, |writer| {
            writer.write_integer(1_634_546_845, 0);
            writer.write_list(1, 1, |writer| {
                writer.write_struct(0, |writer| {
                    writer.write_struct(0, |_| {});
                    writer.write_string("http://videoal-platform.cdn.huya.com/replay-360.m3u8", 1);
                    writer.write_integer(19_637, 2);
                    writer.write_list(3, 2, |writer| {
                        write_definition(
                            writer,
                            "640",
                            "360",
                            "350",
                            "http://videoal-platform.cdn.huya.com/replay-360.m3u8",
                            "流畅",
                        );
                        write_definition(
                            writer,
                            "1920",
                            "1080",
                            "4000",
                            "http://videoal-platform.cdn.huya.com/replay-1080.m3u8",
                            "原画",
                        );
                    });
                });
            });
        });

        let mut params = TarsWriter::default();
        params.write_string_bytes_map("tRsp", &t_rsp.bytes, 0);
        let mut packet = TarsWriter::default();
        packet.write_integer(3, 1);
        packet.write_integer(0, 2);
        packet.write_integer(0, 3);
        packet.write_integer(0, 4);
        packet.write_string("liveui", 5);
        packet.write_string("getVideoHisUpon", 6);
        packet.write_bytes(&params.bytes, 7);
        packet.write_integer(0, 8);
        packet.write_empty_map(9);
        packet.write_empty_map(10);

        let size = packet.bytes.len() + 4;
        let mut response = Vec::with_capacity(size);
        response.extend_from_slice(&(size as u32).to_be_bytes());
        response.extend_from_slice(&packet.bytes);
        response
    }

    fn write_definition(
        writer: &mut TarsWriter,
        width: &str,
        height: &str,
        definition: &str,
        m3u8: &str,
        name: &str,
    ) {
        writer.write_struct(0, |writer| {
            writer.write_string("20", 0);
            writer.write_string(width, 1);
            writer.write_string(height, 2);
            writer.write_string(definition, 3);
            writer.write_string(m3u8, 4);
            writer.write_string(m3u8, 5);
            writer.write_string(name, 6);
            writer.write_string("", 7);
            writer.write_string("0@0", 8);
            writer.write_integer(0, 9);
            writer.write_integer(0, 10);
            writer.write_empty_map(11);
            writer.write_list(12, 0, |_| {});
            writer.write_string("h264", 13);
            writer.write_string("m3u8", 14);
            writer.write_string("25609", 15);
        });
    }
}
