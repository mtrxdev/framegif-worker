use std::path::{Path, PathBuf};

use crate::auto::{
    apply_adjustment, auto_params, clamp_crop, duration_sec, estimate_frames, scale_height,
    working_size,
};
use crate::blob::{ObjectStore, download_source};
use crate::caps::{meets_file_caps, missed_file_caps, pixel_frames};
use crate::encode::GifEncoder;
use crate::gif::Gif;
use crate::hmac_auth::{sign_body, unix_now};
use crate::probe::{probe_gif, probe_source};
use crate::types::{
    ADJUSTMENT_ORDER, CreateJobRequest, EncodeAttempt, EncodeError, EncodeParams, JobCallback,
    JobError, MAX_ENCODE_ATTEMPTS, Measure, ObjectLocator, Probe,
};

pub trait GifProber: Send + Sync {
    fn probe_gif(&self, path: &Path) -> Result<Probe, JobError>;
}

#[derive(Clone, Copy)]
pub struct Ffprobe;

impl GifProber for Ffprobe {
    fn probe_gif(&self, path: &Path) -> Result<Probe, JobError> {
        probe_gif(path)
    }
}

pub struct JobContext<E: GifEncoder, P: GifProber> {
    pub encoder: E,
    pub prober: P,
    pub store: ObjectStore,
    pub client: reqwest::Client,
    pub web_url: Option<String>,
    pub job_secret: String,
}

#[derive(Debug)]
pub struct FileHeld {
    pub path: PathBuf,
    pub measure: Measure,
    pub attempts: Vec<EncodeAttempt>,
    pub params: EncodeParams,
}

pub fn measure_from_gif(params: &crate::types::EncodeParams, probe: &Probe, bytes: u64) -> Measure {
    let frames = probe.nb_frames;
    Measure {
        width: probe.width,
        height: probe.height,
        frames,
        pixels: pixel_frames(probe.width, probe.height, frames),
        fps: probe.fps,
        quality: params.quality,
        bytes,
        format_name: probe.format_name.clone(),
        codec_name: probe.codec_name.clone(),
    }
}

#[allow(clippy::too_many_arguments)]
fn fit<E: GifEncoder, P: GifProber>(
    encoder: &E,
    prober: &P,
    probe: &Probe,
    src: &Path,
    work_dir: &Path,
    start_frame: u32,
    end_frame: u32,
    crop: Option<crate::types::CropRect>,
    prefer_width: Option<u32>,
    prefer_quality: Option<u8>,
) -> Result<FileHeld, EncodeError> {
    let crop = crop.map(|rect| clamp_crop(rect, probe.width, probe.height));
    let mut params = auto_params(
        probe,
        start_frame,
        end_frame,
        crop,
        prefer_width,
        prefer_quality,
    );
    let mut attempts = Vec::new();

    for index in 1..=MAX_ENCODE_ATTEMPTS {
        let (src_w, src_h) = working_size(probe, params.crop);
        params.height = scale_height(params.width, src_w, src_h);
        let duration = duration_sec(params.start_frame, params.end_frame, probe.fps);
        let estimated_frames = estimate_frames(duration, params.fps);
        let estimated_pixels = pixel_frames(params.width, params.height, estimated_frames);
        let dst = work_dir.join(format!("attempt-{index}.gif"));
        let encoded = encoder
            .encode(&params, probe, src, &dst)
            .map_err(JobError::into_encode)?;
        let gif_probe = prober
            .probe_gif(&encoded.path)
            .map_err(|err| EncodeError::Probe(err.to_string()))?;
        let measure = measure_from_gif(&params, &gif_probe, encoded.bytes);
        let missed = missed_file_caps(
            measure.width,
            measure.height,
            measure.frames,
            measure.fps,
            measure.bytes,
            &measure.format_name,
            &measure.codec_name,
        );
        if missed.is_empty() && meets_file_caps(&measure) {
            return Ok(FileHeld {
                path: encoded.path,
                measure,
                attempts,
                params,
            });
        }
        attempts.push(EncodeAttempt {
            index,
            params: params.clone(),
            frames: measure.frames,
            pixels: if measure.pixels == 0 {
                estimated_pixels
            } else {
                measure.pixels
            },
            bytes: Some(measure.bytes),
            missed: missed.clone(),
        });
        let _ = std::fs::remove_file(&encoded.path);
        if index == MAX_ENCODE_ATTEMPTS {
            return Err(EncodeError::FourMisses { attempts });
        }
        let knob = ADJUSTMENT_ORDER[(index as usize) - 1];
        params = apply_adjustment(&params, probe, knob);
    }
    Err(EncodeError::FourMisses { attempts })
}

pub struct Encoded {
    pub gif: Gif,
    pub output_url: String,
    pub output_pathname: String,
    pub attempts: Vec<EncodeAttempt>,
}

pub async fn encode<E: GifEncoder + Clone + 'static, P: GifProber + Clone + 'static>(
    ctx: &JobContext<E, P>,
    request: &CreateJobRequest,
    src: &Path,
    work_dir: &Path,
    probe: &Probe,
) -> Result<Encoded, EncodeError> {
    if request.end_frame < request.start_frame {
        return Err(EncodeError::Range("endFrame must be >= startFrame".into()));
    }
    let encoder = ctx.encoder.clone();
    let prober = ctx.prober.clone();
    let src_clone = src.to_path_buf();
    let work_path = work_dir.to_path_buf();
    let start_frame = request.start_frame;
    let end_frame = request.end_frame;
    let crop = request.crop;
    let prefer_width = request.width;
    let prefer_quality = request.quality;
    let probe_clone = probe.clone();
    let held = tokio::task::spawn_blocking(move || {
        fit(
            &encoder,
            &prober,
            &probe_clone,
            &src_clone,
            &work_path,
            start_frame,
            end_frame,
            crop,
            prefer_width,
            prefer_quality,
        )
    })
    .await
    .map_err(|err| EncodeError::Pipe(format!("join encode: {err}")))??;

    let bytes = tokio::fs::read(&held.path)
        .await
        .map_err(|err| EncodeError::Store(format!("read gif: {err}")))?;
    let pathname = format!("gifs/{}.gif", request.job_id);
    let put = ctx
        .store
        .put(&ctx.client, &pathname, bytes, "image/gif")
        .await
        .map_err(|err| EncodeError::Store(err.to_string()))?;
    let length = ctx
        .store
        .head_length(&ctx.client, &put.url, &pathname)
        .await
        .map_err(|err| EncodeError::Store(err.to_string()))?;
    match Gif::certify(&held.measure, length) {
        Ok(gif) => Ok(Encoded {
            gif: gif.bind(
                ObjectLocator {
                    pathname: put.pathname.clone(),
                    url: put.url.clone(),
                },
                held.attempts.clone(),
            ),
            output_url: put.url,
            output_pathname: put.pathname,
            attempts: held.attempts,
        }),
        Err(err) => {
            let _ = ctx.store.delete(&pathname).await;
            Err(err)
        }
    }
}

pub async fn persist_callback<E: GifEncoder, P: GifProber>(
    ctx: &JobContext<E, P>,
    job_id: &str,
    payload: &JobCallback,
) -> Result<(), JobError> {
    let body = serde_json::to_vec(payload).map_err(|err| JobError::msg(err.to_string()))?;
    let pathname = format!("jobs/{job_id}.callback.json");
    ctx.store
        .put(&ctx.client, &pathname, body, "application/json")
        .await?;
    callback_web(ctx, job_id, payload).await
}

pub async fn callback_web<E: GifEncoder, P: GifProber>(
    ctx: &JobContext<E, P>,
    job_id: &str,
    payload: &JobCallback,
) -> Result<(), JobError> {
    let Some(web_url) = ctx.web_url.as_ref() else {
        return Ok(());
    };
    let body = serde_json::to_vec(payload).map_err(|err| JobError::msg(err.to_string()))?;
    let ts = unix_now();
    let sig = sign_body(&ctx.job_secret, ts, &body);
    let url = format!("{web_url}/api/internal/jobs/{job_id}");
    let response = ctx
        .client
        .post(url)
        .header("x-timestamp", ts.to_string())
        .header("x-signature", sig)
        .header("content-type", "application/json")
        .body(body)
        .send()
        .await
        .map_err(|err| JobError::msg(format!("callback: {err}")))?;
    if !response.status().is_success() {
        return Err(JobError::msg(format!("callback {}", response.status())));
    }
    Ok(())
}

fn error_callback(error: &EncodeError, attempts: Vec<EncodeAttempt>) -> JobCallback {
    JobCallback {
        status: "error".into(),
        error: Some(error.message()),
        output_url: None,
        output_pathname: None,
        measure: None,
        attempts,
    }
}

pub async fn run_job<E: GifEncoder + Clone + 'static, P: GifProber + Clone + 'static>(
    ctx: &JobContext<E, P>,
    request: CreateJobRequest,
) -> Result<Gif, EncodeError> {
    let _ = persist_callback(
        ctx,
        &request.job_id,
        &JobCallback {
            status: "processing".into(),
            error: None,
            output_url: None,
            output_pathname: None,
            measure: None,
            attempts: vec![],
        },
    )
    .await;

    let work = tempfile::tempdir().map_err(|err| EncodeError::Store(format!("tempdir: {err}")))?;
    let src = work.path().join("source");
    download_source(
        &ctx.client,
        &ctx.store,
        &request.source_url,
        &request.source_pathname,
        &src,
    )
    .await
    .map_err(|err| EncodeError::Source(err.to_string()))?;
    let probe = probe_source(&src).map_err(|err| EncodeError::Probe(err.to_string()))?;

    match encode(ctx, &request, &src, work.path(), &probe).await {
        Ok(encoded) => {
            persist_callback(
                ctx,
                &request.job_id,
                &JobCallback {
                    status: "done".into(),
                    error: None,
                    output_url: Some(encoded.output_url.clone()),
                    output_pathname: Some(encoded.output_pathname.clone()),
                    measure: Some(encoded.gif.measure()),
                    attempts: encoded.attempts.clone(),
                },
            )
            .await
            .map_err(|err| EncodeError::Store(err.to_string()))?;
            Ok(encoded.gif)
        }
        Err(err @ EncodeError::FourMisses { attempts: _ }) => {
            let attempts = match &err {
                EncodeError::FourMisses { attempts } => attempts.clone(),
                _ => vec![],
            };
            let _ = persist_callback(ctx, &request.job_id, &error_callback(&err, attempts)).await;
            Err(err)
        }
        Err(err) => {
            let _ = persist_callback(ctx, &request.job_id, &error_callback(&err, vec![])).await;
            Err(err)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::encode::{EncodeOutput, GifEncoder};
    use crate::probe::probe_from_json;
    use crate::types::{EncodeParams, MAX_BYTES};
    use std::path::Path;
    use std::sync::atomic::{AtomicU8, Ordering};

    #[derive(Clone)]
    struct ScriptedEncoder {
        remaining_oversize: std::sync::Arc<AtomicU8>,
    }

    impl GifEncoder for ScriptedEncoder {
        fn encode(
            &self,
            params: &EncodeParams,
            _probe: &Probe,
            _src: &Path,
            dst: &Path,
        ) -> Result<EncodeOutput, JobError> {
            if let Some(parent) = dst.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            let oversize = self.remaining_oversize.load(Ordering::SeqCst) > 0;
            if oversize {
                self.remaining_oversize.fetch_sub(1, Ordering::SeqCst);
            }
            let bytes: u64 = if oversize { MAX_BYTES + 12 } else { 2048 };
            std::fs::write(dst, vec![0u8; 16]).unwrap();
            let gif_json = format!(
                r#"{{"streams":[{{"codec_name":"gif","width":{},"height":{},"avg_frame_rate":"10/1","nb_read_frames":"10"}}],"format":{{"format_name":"gif","size":"{bytes}"}}}}"#,
                params.width,
                params.height.max(2)
            );
            std::fs::write(dst.with_extension("probe.json"), gif_json).unwrap();
            Ok(EncodeOutput {
                path: dst.to_path_buf(),
                bytes,
            })
        }
    }

    #[derive(Clone)]
    struct SidecarProber;

    impl GifProber for SidecarProber {
        fn probe_gif(&self, path: &Path) -> Result<Probe, JobError> {
            let json = std::fs::read_to_string(path.with_extension("probe.json"))
                .map_err(|err| JobError::msg(err.to_string()))?;
            probe_from_json(&json)
        }
    }

    fn tiny_probe() -> Probe {
        Probe {
            width: 64,
            height: 64,
            fps: 10.0,
            duration_sec: 1.0,
            nb_frames: 10,
            codec_name: "h264".into(),
            format_name: "mp4".into(),
        }
    }

    #[test]
    fn four_misses_leaves_no_object() {
        let encoder = ScriptedEncoder {
            remaining_oversize: std::sync::Arc::new(AtomicU8::new(4)),
        };
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src");
        std::fs::write(&src, b"x").unwrap();
        let err = fit(
            &encoder,
            &SidecarProber,
            &tiny_probe(),
            &src,
            dir.path(),
            0,
            9,
            None,
            None,
            None,
        )
        .unwrap_err();
        assert!(matches!(err, EncodeError::FourMisses { ref attempts } if attempts.len() == 4));
        let gifs: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| e.path().extension().is_some_and(|ext| ext == "gif"))
            .collect();
        assert!(gifs.is_empty());
    }

    #[test]
    fn retry_order_is_width_then_fps() {
        let encoder = ScriptedEncoder {
            remaining_oversize: std::sync::Arc::new(AtomicU8::new(2)),
        };
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src");
        std::fs::write(&src, b"x").unwrap();
        let held = fit(
            &encoder,
            &SidecarProber,
            &tiny_probe(),
            &src,
            dir.path(),
            0,
            9,
            None,
            None,
            None,
        )
        .unwrap();
        assert_eq!(held.attempts.len(), 2);
        assert!(held.attempts[0].params.width > held.attempts[1].params.width);
        assert!(held.attempts[1].params.fps > held.params.fps);
        assert!(held.path.exists());
        assert!(held.measure.bytes <= MAX_BYTES);
    }
}
