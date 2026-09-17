use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::auto::{end_timestamp, ffmpeg_filter, start_timestamp};
use crate::types::{EncodeParams, JobError, Probe};

pub struct EncodeOutput {
    pub path: PathBuf,
    pub bytes: u64,
}

pub trait GifEncoder: Send + Sync {
    fn encode(
        &self,
        params: &EncodeParams,
        probe: &Probe,
        src: &Path,
        dst: &Path,
    ) -> Result<EncodeOutput, JobError>;
}

#[derive(Clone)]
pub struct FfmpegGifski {
    pub ffmpeg: String,
    pub gifski: String,
}

impl Default for FfmpegGifski {
    fn default() -> Self {
        Self {
            ffmpeg: std::env::var("FFMPEG_BIN").unwrap_or_else(|_| "ffmpeg".into()),
            gifski: std::env::var("GIFSKI_BIN").unwrap_or_else(|_| "gifski".into()),
        }
    }
}

pub fn ffmpeg_args(
    params: &EncodeParams,
    probe: &Probe,
    src: &Path,
) -> Result<Vec<String>, JobError> {
    let start = start_timestamp(params.start_frame, probe.fps);
    let end = end_timestamp(params.end_frame, probe.fps);
    if end <= start {
        return Err(JobError::msg("end must be after start"));
    }
    let src_str = src
        .to_str()
        .ok_or_else(|| JobError::msg("source path is not utf-8"))?;
    Ok(vec![
        "-hide_banner".into(),
        "-loglevel".into(),
        "error".into(),
        "-ss".into(),
        format!("{start:.6}"),
        "-to".into(),
        format!("{end:.6}"),
        "-i".into(),
        src_str.to_string(),
        "-an".into(),
        "-vf".into(),
        ffmpeg_filter(params),
        "-f".into(),
        "yuv4mpegpipe".into(),
        "-".into(),
    ])
}

pub fn gifski_args(params: &EncodeParams, dst: &Path) -> Result<Vec<String>, JobError> {
    let dst_str = dst
        .to_str()
        .ok_or_else(|| JobError::msg("output path is not utf-8"))?;
    Ok(vec![
        "--extra".into(),
        "--quality".into(),
        params.quality.to_string(),
        "--fps".into(),
        format!("{}", params.fps),
        "--width".into(),
        params.width.to_string(),
        "-o".into(),
        dst_str.to_string(),
        "-".into(),
    ])
}

impl GifEncoder for FfmpegGifski {
    fn encode(
        &self,
        params: &EncodeParams,
        probe: &Probe,
        src: &Path,
        dst: &Path,
    ) -> Result<EncodeOutput, JobError> {
        if let Some(parent) = dst.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|err| JobError::msg(format!("create output dir: {err}")))?;
        }
        let ff_args = ffmpeg_args(params, probe, src)?;
        let gk_args = gifski_args(params, dst)?;
        let mut ffmpeg = Command::new(&self.ffmpeg)
            .args(&ff_args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|err| JobError::msg(format!("spawn ffmpeg: {err}")))?;
        let stdout = ffmpeg
            .stdout
            .take()
            .ok_or_else(|| JobError::msg("ffmpeg stdout missing"))?;
        let gifski = Command::new(&self.gifski)
            .args(&gk_args)
            .stdin(stdout)
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|err| JobError::msg(format!("spawn gifski: {err}")))?;

        let mut ff_err = String::new();
        if let Some(mut stderr) = ffmpeg.stderr.take() {
            let mut buf = Vec::new();
            let _ = stderr.read_to_end(&mut buf);
            ff_err = String::from_utf8_lossy(&buf).into_owned();
        }
        let gk_out = gifski
            .wait_with_output()
            .map_err(|err| JobError::msg(format!("wait gifski: {err}")))?;
        let ff_status = ffmpeg
            .wait()
            .map_err(|err| JobError::msg(format!("wait ffmpeg: {err}")))?;

        if !ff_status.success() {
            return Err(JobError::msg(format!(
                "ffmpeg exited {}: {ff_err}",
                ff_status.code().unwrap_or(-1)
            )));
        }
        if !gk_out.status.success() {
            return Err(JobError::msg(format!(
                "gifski exited {}: {}",
                gk_out.status.code().unwrap_or(-1),
                String::from_utf8_lossy(&gk_out.stderr)
            )));
        }

        let meta =
            std::fs::metadata(dst).map_err(|err| JobError::msg(format!("stat gif: {err}")))?;
        Ok(EncodeOutput {
            path: dst.to_path_buf(),
            bytes: meta.len(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{EncodeParams, Probe};
    use std::path::Path;

    fn sample_params() -> EncodeParams {
        EncodeParams {
            width: 1080,
            height: 606,
            fps: 20.0,
            quality: 90,
            start_frame: 0,
            end_frame: 59,
            crop: None,
        }
    }

    fn sample_probe() -> Probe {
        Probe {
            width: 1920,
            height: 1080,
            fps: 30.0,
            duration_sec: 2.0,
            nb_frames: 60,
            codec_name: "h264".into(),
            format_name: "mp4".into(),
        }
    }

    #[test]
    fn ffmpeg_pipeline_order() {
        let args =
            ffmpeg_args(&sample_params(), &sample_probe(), Path::new("/tmp/src.mp4")).unwrap();
        assert_eq!(args[3], "-ss");
        assert_eq!(args[5], "-to");
        assert_eq!(args[7], "-i");
        assert_eq!(args[8], "/tmp/src.mp4");
        assert_eq!(args[9], "-an");
        assert_eq!(args[10], "-vf");
        assert_eq!(
            args[11],
            "scale=1080:-2:flags=lanczos+accurate_rnd+full_chroma_int"
        );
        assert_eq!(args[12], "-f");
        assert_eq!(args[13], "yuv4mpegpipe");
        assert_eq!(args[14], "-");
    }

    #[test]
    fn gifski_flags() {
        let args = gifski_args(&sample_params(), Path::new("/tmp/out.gif")).unwrap();
        assert_eq!(
            args,
            [
                "--extra",
                "--quality",
                "90",
                "--fps",
                "20",
                "--width",
                "1080",
                "-o",
                "/tmp/out.gif",
                "-"
            ]
        );
    }
}
