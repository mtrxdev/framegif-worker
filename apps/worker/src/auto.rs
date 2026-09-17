use crate::caps::pixel_frames;
use crate::types::{
    Adjustment, CropRect, DEFAULT_QUALITY, EncodeParams, FIT_BOX_HEIGHT, FIT_BOX_WIDTH, MAX_FPS,
    MAX_FRAMES, MAX_HEIGHT, MAX_PIXEL_FRAMES, MAX_WIDTH, MIN_FPS, MIN_QUALITY, MIN_WIDTH, Probe,
};

pub fn even_floor(value: f64) -> u32 {
    if !value.is_finite() || value <= 0.0 {
        return 0;
    }
    let floored = value.floor() as u32;
    floored - (floored % 2)
}

pub fn scale_height(width: u32, src_width: u32, src_height: u32) -> u32 {
    if src_width == 0 {
        return 0;
    }
    2 * ((((f64::from(src_height) / f64::from(src_width)) * f64::from(width)) / 2.0).floor() as u32)
}

pub fn working_size(probe: &Probe, crop: Option<CropRect>) -> (u32, u32) {
    match crop {
        Some(rect) => (rect.width, rect.height),
        None => (probe.width, probe.height),
    }
}

pub fn fit_width(src_width: u32, src_height: u32) -> u32 {
    if src_width == 0 || src_height == 0 {
        return 0;
    }
    let scale = (f64::from(FIT_BOX_WIDTH) / f64::from(src_width))
        .min(f64::from(FIT_BOX_HEIGHT) / f64::from(src_height))
        .min(1.0);
    let mut width = even_floor(f64::from(src_width) * scale).max(2);
    let mut height = scale_height(width, src_width, src_height);
    while (width > MAX_WIDTH || height > MAX_HEIGHT) && width >= 2 {
        if width > MAX_WIDTH {
            width = even_floor(f64::from(width.min(MAX_WIDTH)));
        }
        height = scale_height(width, src_width, src_height);
        if height > MAX_HEIGHT {
            width = even_floor(f64::from(width.saturating_sub(2)));
            height = scale_height(width, src_width, src_height);
        }
    }
    width
}

pub fn inclusive_frame_count(start_frame: u32, end_frame: u32) -> u32 {
    end_frame.saturating_sub(start_frame).saturating_add(1)
}

pub fn duration_sec(start_frame: u32, end_frame: u32, src_fps: f64) -> f64 {
    if src_fps <= 0.0 {
        return 0.0;
    }
    f64::from(inclusive_frame_count(start_frame, end_frame)) / src_fps
}

pub fn estimate_frames(duration: f64, out_fps: f64) -> u32 {
    if duration <= 0.0 || out_fps <= 0.0 {
        return 1;
    }
    (duration * out_fps).round().max(1.0) as u32
}

pub fn start_timestamp(start_frame: u32, src_fps: f64) -> f64 {
    if src_fps <= 0.0 {
        return 0.0;
    }
    f64::from(start_frame) / src_fps
}

pub fn end_timestamp(end_frame: u32, src_fps: f64) -> f64 {
    if src_fps <= 0.0 {
        return 0.0;
    }
    f64::from(end_frame.saturating_add(1)) / src_fps
}

pub fn clamp_crop(crop: CropRect, src_width: u32, src_height: u32) -> CropRect {
    let x = even_floor(f64::from(crop.x));
    let y = even_floor(f64::from(crop.y));
    let mut width = even_floor(f64::from(crop.width));
    let mut height = even_floor(f64::from(crop.height));
    if x + width > src_width {
        width = even_floor(f64::from(src_width.saturating_sub(x)));
    }
    if y + height > src_height {
        height = even_floor(f64::from(src_height.saturating_sub(y)));
    }
    CropRect {
        x,
        y,
        width: width.max(2),
        height: height.max(2),
    }
}

fn clamp_fps(fps: f64) -> f64 {
    if !fps.is_finite() || fps <= 0.0 {
        return MIN_FPS;
    }
    fps.clamp(MIN_FPS, MAX_FPS)
}

pub fn ffmpeg_filter(params: &EncodeParams) -> String {
    let scale = format!(
        "scale={}:-2:flags=lanczos+accurate_rnd+full_chroma_int",
        params.width
    );
    match params.crop {
        None => scale,
        Some(crop) => format!(
            "crop={}:{}:{}:{},{scale}",
            crop.width, crop.height, crop.x, crop.y
        ),
    }
}

pub fn auto_params(
    probe: &Probe,
    start_frame: u32,
    end_frame: u32,
    crop: Option<CropRect>,
    prefer_width: Option<u32>,
    prefer_quality: Option<u8>,
) -> EncodeParams {
    let (src_w, src_h) = working_size(probe, crop);
    let mut width = fit_width(src_w, src_h);
    if let Some(preferred) = prefer_width {
        let max_w = width;
        width = even_floor(f64::from(preferred.clamp(MIN_WIDTH, max_w.max(MIN_WIDTH))));
        if width < 2 {
            width = 2;
        }
    }
    let mut height = scale_height(width, src_w, src_h);
    let start = start_frame;
    let mut end = end_frame.max(start);
    let src_fps = if probe.fps > 0.0 { probe.fps } else { 30.0 };
    let mut duration = duration_sec(start, end, src_fps);
    let mut fps = clamp_fps(src_fps.min(20.0).min(MAX_FPS));
    let mut frames = estimate_frames(duration, fps);

    if frames > MAX_FRAMES {
        fps = clamp_fps(f64::from(MAX_FRAMES) / duration);
        frames = estimate_frames(duration, fps);
    }

    if frames > MAX_FRAMES {
        let max_duration = f64::from(MAX_FRAMES) / fps;
        let max_count = (max_duration * src_fps).floor().max(1.0) as u32;
        end = start.saturating_add(max_count).saturating_sub(1);
        duration = duration_sec(start, end, src_fps);
        frames = estimate_frames(duration, fps);
    }

    while pixel_frames(width, height, frames) > MAX_PIXEL_FRAMES && width > MIN_WIDTH {
        width = even_floor((f64::from(width) * 0.9).max(f64::from(MIN_WIDTH)));
        height = scale_height(width, src_w, src_h);
    }

    if pixel_frames(width, height, frames) > MAX_PIXEL_FRAMES {
        let max_frames = (MAX_PIXEL_FRAMES / u64::from(width.max(1) * height.max(1))).max(1);
        fps = clamp_fps(max_frames as f64 / duration);
        frames = estimate_frames(duration, fps);
        if u64::from(frames) > max_frames {
            let max_duration = max_frames as f64 / fps;
            let max_count = (max_duration * src_fps).floor().max(1.0) as u32;
            end = start.saturating_add(max_count).saturating_sub(1);
            duration = duration_sec(start, end, src_fps);
            frames = estimate_frames(duration, fps);
        }
    }

    debug_assert!(frames > 0);

    EncodeParams {
        width,
        height: scale_height(width, src_w, src_h),
        fps,
        quality: prefer_quality
            .map(|value| value.clamp(MIN_QUALITY, 100))
            .unwrap_or(DEFAULT_QUALITY),
        start_frame: start,
        end_frame: end,
        crop,
    }
}

pub fn apply_adjustment(params: &EncodeParams, probe: &Probe, knob: Adjustment) -> EncodeParams {
    let (src_w, src_h) = working_size(probe, params.crop);
    let mut next = params.clone();
    match knob {
        Adjustment::Width => {
            let reduced = if params.width <= MIN_WIDTH {
                even_floor(f64::from(params.width.saturating_sub(2)))
            } else {
                even_floor((f64::from(params.width) * 0.82).max(f64::from(MIN_WIDTH)))
            };
            next.width = reduced.max(2);
            if next.width >= params.width {
                next.width = even_floor(f64::from(params.width.saturating_sub(2))).max(2);
            }
            next.height = scale_height(next.width, src_w, src_h);
        }
        Adjustment::Fps => {
            let reduced = clamp_fps(params.fps * 0.75);
            next.fps = if (reduced - params.fps).abs() < f64::EPSILON {
                clamp_fps(params.fps - 1.0)
            } else {
                reduced
            };
        }
        Adjustment::Quality => {
            next.quality = params.quality.saturating_sub(20).max(MIN_QUALITY);
        }
    }
    let duration = duration_sec(next.start_frame, next.end_frame, probe.fps);
    let frames = estimate_frames(duration, next.fps);
    if pixel_frames(next.width, next.height, frames) > MAX_PIXEL_FRAMES && knob != Adjustment::Width
    {
        return apply_adjustment(&next, probe, Adjustment::Width);
    }
    next
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hd() -> Probe {
        Probe {
            width: 1920,
            height: 1080,
            fps: 30.0,
            duration_sec: 12.0,
            nb_frames: 360,
            codec_name: "h264".into(),
            format_name: "mp4".into(),
        }
    }

    #[test]
    fn sixteen_by_nine_1080p_becomes_1080x606() {
        let width = fit_width(1920, 1080);
        assert_eq!(width, 1080);
        assert_eq!(scale_height(1080, 1920, 1080), 606);
    }

    #[test]
    fn portrait_fits_height_cap() {
        let width = fit_width(1080, 1920);
        let height = scale_height(width, 1080, 1920);
        assert!(width <= 1080);
        assert!(height <= 1080);
        assert_eq!(width % 2, 0);
        assert_eq!(height % 2, 0);
    }

    #[test]
    fn auto_holds_geometry_caps() {
        let probe = hd();
        let params = auto_params(&probe, 0, 359, None, None, None);
        let duration = duration_sec(params.start_frame, params.end_frame, probe.fps);
        let frames = estimate_frames(duration, params.fps);
        assert!(params.width <= 1080);
        assert!(params.height <= 1080);
        assert!(frames <= 350);
        assert!(pixel_frames(params.width, params.height, frames) <= MAX_PIXEL_FRAMES);
        assert!(params.fps <= 50.0);
        assert_eq!(params.quality, 90);
    }

    #[test]
    fn filter_omits_crop_until_supplied() {
        let params = auto_params(&hd(), 0, 100, None, None, None);
        assert_eq!(
            ffmpeg_filter(&params),
            format!(
                "scale={}:-2:flags=lanczos+accurate_rnd+full_chroma_int",
                params.width
            )
        );
        let mut cropped = params.clone();
        cropped.crop = Some(CropRect {
            x: 10,
            y: 20,
            width: 800,
            height: 800,
        });
        assert!(ffmpeg_filter(&cropped).starts_with("crop=800:800:10:20,scale="));
    }

    #[test]
    fn adjustment_order_width_fps_quality() {
        let probe = hd();
        let base = auto_params(&probe, 0, 299, None, None, None);
        let width = apply_adjustment(&base, &probe, Adjustment::Width);
        assert!(width.width < base.width);
        let fps = apply_adjustment(&width, &probe, Adjustment::Fps);
        assert!(fps.fps < width.fps);
        let quality = apply_adjustment(&fps, &probe, Adjustment::Quality);
        assert_eq!(quality.quality, 70);
    }

    #[test]
    fn prefers_width_and_quality() {
        let params = auto_params(&hd(), 0, 100, None, Some(720), Some(70));
        assert_eq!(params.width, 720);
        assert_eq!(params.height, scale_height(720, 1920, 1080));
        assert_eq!(params.quality, 70);
    }
}
