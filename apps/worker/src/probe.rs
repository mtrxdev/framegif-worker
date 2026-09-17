use std::path::Path;
use std::process::Command;

use serde::Deserialize;

use crate::types::{JobError, Probe};

#[derive(Debug, Deserialize)]
struct ProbeJson {
    streams: Option<Vec<StreamJson>>,
    format: Option<FormatJson>,
}

#[derive(Debug, Deserialize)]
struct StreamJson {
    codec_name: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
    avg_frame_rate: Option<String>,
    r_frame_rate: Option<String>,
    nb_frames: Option<String>,
    nb_read_frames: Option<String>,
    duration: Option<String>,
}

#[derive(Debug, Deserialize)]
struct FormatJson {
    format_name: Option<String>,
    duration: Option<String>,
    size: Option<String>,
}

pub fn parse_ratio(raw: &str) -> Option<f64> {
    if raw.is_empty() || raw == "0/0" || raw == "N/A" {
        return None;
    }
    if let Some((num, den)) = raw.split_once('/') {
        let n: f64 = num.parse().ok()?;
        let d: f64 = den.parse().ok()?;
        if d == 0.0 {
            return None;
        }
        return Some(n / d);
    }
    raw.parse().ok()
}

pub fn parse_u32(raw: Option<&str>) -> Option<u32> {
    let value = raw?;
    if value == "N/A" {
        return None;
    }
    value.parse().ok()
}

pub fn parse_u64(raw: Option<&str>) -> Option<u64> {
    let value = raw?;
    if value == "N/A" {
        return None;
    }
    value.parse().ok()
}

pub fn parse_f64(raw: Option<&str>) -> Option<f64> {
    let value = raw?;
    if value == "N/A" {
        return None;
    }
    value.parse().ok()
}

pub fn probe_from_json(json: &str) -> Result<Probe, JobError> {
    let parsed: ProbeJson =
        serde_json::from_str(json).map_err(|err| JobError::msg(format!("ffprobe json: {err}")))?;
    let stream = parsed
        .streams
        .and_then(|mut streams| streams.drain(..).next())
        .ok_or_else(|| JobError::msg("ffprobe: no video stream"))?;
    let format = parsed.format.unwrap_or(FormatJson {
        format_name: None,
        duration: None,
        size: None,
    });
    let width = stream
        .width
        .ok_or_else(|| JobError::msg("ffprobe: missing width"))?;
    let height = stream
        .height
        .ok_or_else(|| JobError::msg("ffprobe: missing height"))?;
    let fps = stream
        .avg_frame_rate
        .as_deref()
        .and_then(parse_ratio)
        .or_else(|| stream.r_frame_rate.as_deref().and_then(parse_ratio))
        .filter(|fps| *fps > 0.0)
        .ok_or_else(|| JobError::msg("ffprobe: missing fps"))?;
    let duration = parse_f64(stream.duration.as_deref())
        .or_else(|| parse_f64(format.duration.as_deref()))
        .unwrap_or(0.0);
    let nb_frames = parse_u32(stream.nb_read_frames.as_deref())
        .or_else(|| parse_u32(stream.nb_frames.as_deref()))
        .unwrap_or_else(|| {
            if duration > 0.0 {
                (duration * fps).round().max(1.0) as u32
            } else {
                1
            }
        });
    Ok(Probe {
        width,
        height,
        fps,
        duration_sec: duration,
        nb_frames,
        codec_name: stream.codec_name.unwrap_or_default(),
        format_name: format.format_name.unwrap_or_default(),
    })
}

#[cfg_attr(not(test), allow(dead_code))]
pub fn format_size_bytes(json: &str) -> Result<u64, JobError> {
    let parsed: ProbeJson =
        serde_json::from_str(json).map_err(|err| JobError::msg(format!("ffprobe json: {err}")))?;
    parsed
        .format
        .and_then(|format| parse_u64(format.size.as_deref()))
        .ok_or_else(|| JobError::msg("ffprobe: missing format size"))
}

fn ffprobe_bin() -> String {
    std::env::var("FFPROBE_BIN").unwrap_or_else(|_| "ffprobe".into())
}

pub fn run_ffprobe(path: &Path, count_frames: bool) -> Result<String, JobError> {
    let path_str = path
        .to_str()
        .ok_or_else(|| JobError::msg("probe path is not utf-8"))?;
    let mut cmd = Command::new(ffprobe_bin());
    cmd.args([
        "-v",
        "error",
        "-print_format",
        "json",
        "-select_streams",
        "v:0",
    ]);
    if count_frames {
        cmd.arg("-count_frames");
        cmd.args([
            "-show_entries",
            "stream=width,height,nb_read_frames,nb_frames,codec_name,avg_frame_rate,r_frame_rate,duration:format=format_name,size,duration",
        ]);
    } else {
        cmd.args(["-show_streams", "-show_format"]);
    }
    cmd.arg(path_str);
    let output = cmd
        .output()
        .map_err(|err| JobError::msg(format!("spawn ffprobe: {err}")))?;
    if !output.status.success() {
        return Err(JobError::msg(format!(
            "ffprobe exited {}: {}",
            output.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    String::from_utf8(output.stdout).map_err(|err| JobError::msg(format!("ffprobe utf8: {err}")))
}

pub fn probe_source(path: &Path) -> Result<Probe, JobError> {
    probe_from_json(&run_ffprobe(path, false)?)
}

pub fn probe_gif(path: &Path) -> Result<Probe, JobError> {
    probe_from_json(&run_ffprobe(path, true)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_gif_probe() {
        let json = r#"{
            "streams": [{
                "codec_name": "gif",
                "width": 320,
                "height": 180,
                "avg_frame_rate": "20/1",
                "nb_read_frames": "40"
            }],
            "format": { "format_name": "gif", "size": "12345", "duration": "2.000000" }
        }"#;
        let probe = probe_from_json(json).unwrap();
        assert_eq!(probe.width, 320);
        assert_eq!(probe.height, 180);
        assert_eq!(probe.fps, 20.0);
        assert_eq!(probe.nb_frames, 40);
        assert_eq!(probe.codec_name, "gif");
        assert_eq!(probe.format_name, "gif");
        assert_eq!(format_size_bytes(json).unwrap(), 12345);
    }

    #[test]
    fn parses_fractional_fps() {
        assert!((parse_ratio("30000/1001").unwrap() - 29.970).abs() < 0.01);
    }
}
