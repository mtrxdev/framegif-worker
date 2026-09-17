use serde::{Deserialize, Serialize};

pub const MAX_WIDTH: u32 = 1080;
pub const MAX_HEIGHT: u32 = 1080;
pub const MAX_FRAMES: u32 = 350;
pub const MAX_PIXEL_FRAMES: u64 = 300_000_000;
pub const MAX_BYTES: u64 = 15_728_640;
pub const MAX_FPS: f64 = 50.0;
pub const FIT_BOX_WIDTH: u32 = 1280;
pub const FIT_BOX_HEIGHT: u32 = 1080;
pub const MAX_ENCODE_ATTEMPTS: u8 = 4;
pub const DEFAULT_QUALITY: u8 = 90;
pub const MIN_QUALITY: u8 = 40;
pub const MIN_WIDTH: u32 = 160;
pub const MIN_FPS: f64 = 8.0;
pub const GIFSKI_VERSION: &str = "1.34.0";

pub const ADJUSTMENT_ORDER: [Adjustment; 3] =
    [Adjustment::Width, Adjustment::Fps, Adjustment::Quality];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Adjustment {
    Width,
    Fps,
    Quality,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CropRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Probe {
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub duration_sec: f64,
    pub nb_frames: u32,
    pub codec_name: String,
    pub format_name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EncodeParams {
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub quality: u8,
    pub start_frame: u32,
    pub end_frame: u32,
    pub crop: Option<CropRect>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Measure {
    pub width: u32,
    pub height: u32,
    pub frames: u32,
    pub pixels: u64,
    pub fps: f64,
    pub quality: u8,
    pub bytes: u64,
    pub format_name: String,
    pub codec_name: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ObjectLocator {
    pub pathname: String,
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EncodeAttempt {
    pub index: u8,
    pub params: EncodeParams,
    pub frames: u32,
    pub pixels: u64,
    pub bytes: Option<u64>,
    pub missed: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateJobRequest {
    pub job_id: String,
    pub source_url: String,
    pub source_pathname: String,
    pub start_frame: u32,
    pub end_frame: u32,
    pub crop: Option<CropRect>,
    #[serde(default)]
    pub width: Option<u32>,
    #[serde(default)]
    pub quality: Option<u8>,
    #[serde(default)]
    pub callback_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobCallback {
    pub status: String,
    pub error: Option<String>,
    pub output_url: Option<String>,
    pub output_pathname: Option<String>,
    pub measure: Option<Measure>,
    pub attempts: Vec<EncodeAttempt>,
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum EncodeError {
    #[error("four misses: job error, no object")]
    FourMisses { attempts: Vec<EncodeAttempt> },
    #[error("{0}")]
    Probe(String),
    #[error("{0}")]
    Pipe(String),
    #[error("{0}")]
    Store(String),
    #[error("{0}")]
    Source(String),
    #[error("stored Content-Length failed caps after HEAD")]
    Head { local: u64, stored: Option<u64> },
    #[error("{0}")]
    Range(String),
}

impl EncodeError {
    pub fn message(&self) -> String {
        self.to_string()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum JobError {
    #[error("{0}")]
    Message(String),
    #[error("four misses: job error, no object")]
    FourMisses(Vec<EncodeAttempt>),
}

impl JobError {
    pub fn msg(text: impl Into<String>) -> Self {
        Self::Message(text.into())
    }

    pub fn into_encode(self) -> EncodeError {
        match self {
            Self::FourMisses(attempts) => EncodeError::FourMisses { attempts },
            Self::Message(text) => EncodeError::Pipe(text),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_job_request_camel_case() {
        let json = r#"{"jobId":"abc","sourceUrl":"https://x/src","sourcePathname":"sources/a","startFrame":12,"endFrame":48,"crop":null}"#;
        let request: CreateJobRequest = serde_json::from_str(json).unwrap();
        assert_eq!(request.job_id, "abc");
        assert_eq!(request.start_frame, 12);
        assert_eq!(request.end_frame, 48);
        assert_eq!(request.width, None);
        assert_eq!(request.quality, None);
        assert_eq!(request.callback_url, None);
    }

    #[test]
    fn cap_literals_match_shared() {
        assert_eq!(MAX_WIDTH, 1080);
        assert_eq!(MAX_HEIGHT, 1080);
        assert_eq!(MAX_FRAMES, 350);
        assert_eq!(MAX_PIXEL_FRAMES, 300_000_000);
        assert_eq!(MAX_BYTES, 15_728_640);
        assert_eq!(MAX_ENCODE_ATTEMPTS, 4);
        assert_eq!(ADJUSTMENT_ORDER.len(), 3);
    }
}
