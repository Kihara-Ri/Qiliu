use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamQualityOption {
    pub value: u32,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamLineOption {
    pub index: usize,
    pub label: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LiveStream {
    pub platform: String,
    pub platform_label: String,
    pub source_url: String,
    pub room_id: String,
    pub title: String,
    pub anchor: String,
    pub avatar_url: String,
    pub cover_url: String,
    pub is_live: bool,
    pub is_replay: bool,
    pub url: Option<String>,
    pub line_index: usize,
    pub line_count: usize,
    pub line_name: String,
    pub line_options: Vec<StreamLineOption>,
    pub quality_label: String,
    pub quality_options: Vec<StreamQualityOption>,
    pub bitrate: u32,
    pub start_position_seconds: f64,
    pub format: String,
}
