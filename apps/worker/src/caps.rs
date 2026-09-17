use crate::types::{
    MAX_BYTES, MAX_FPS, MAX_FRAMES, MAX_HEIGHT, MAX_PIXEL_FRAMES, MAX_WIDTH, Measure,
};

pub fn pixel_frames(width: u32, height: u32, frames: u32) -> u64 {
    u64::from(width) * u64::from(height) * u64::from(frames)
}

pub fn is_gif_format(format_name: &str, codec_name: &str) -> bool {
    format_name
        .to_ascii_lowercase()
        .split(',')
        .any(|part| part == "gif")
        && codec_name.eq_ignore_ascii_case("gif")
}

pub fn missed_file_caps(
    width: u32,
    height: u32,
    frames: u32,
    fps: f64,
    bytes: u64,
    format_name: &str,
    codec_name: &str,
) -> Vec<String> {
    let mut missed = Vec::new();
    if width == 0 || width > MAX_WIDTH {
        missed.push("width".into());
    }
    if height == 0 || height > MAX_HEIGHT {
        missed.push("height".into());
    }
    if frames == 0 || frames > MAX_FRAMES {
        missed.push("frames".into());
    }
    if pixel_frames(width, height, frames) > MAX_PIXEL_FRAMES {
        missed.push("pixelFrames".into());
    }
    if !(fps > 0.0 && fps <= MAX_FPS) {
        missed.push("fps".into());
    }
    if bytes == 0 || bytes > MAX_BYTES {
        missed.push("bytes".into());
    }
    if !is_gif_format(format_name, codec_name) {
        missed.push("format".into());
    }
    missed
}

pub fn meets_file_caps(measure: &Measure) -> bool {
    missed_file_caps(
        measure.width,
        measure.height,
        measure.frames,
        measure.fps,
        measure.bytes,
        &measure.format_name,
        &measure.codec_name,
    )
    .is_empty()
}

pub fn missed_stored_caps(local_bytes: u64, content_length: Option<u64>) -> Vec<String> {
    let mut missed = Vec::new();
    if local_bytes == 0 || local_bytes > MAX_BYTES {
        missed.push("bytes".into());
    }
    match content_length {
        None => missed.push("contentLength".into()),
        Some(length) => {
            if length != local_bytes {
                missed.push("contentLengthMismatch".into());
            }
            if length > MAX_BYTES {
                missed.push("storedBytes".into());
            }
        }
    }
    missed
}

pub fn meets_stored_caps(local_bytes: u64, content_length: Option<u64>) -> bool {
    missed_stored_caps(local_bytes, content_length).is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn square_1080_350_exceeds_pixel_cap() {
        assert!(pixel_frames(1080, 1080, 350) > MAX_PIXEL_FRAMES);
        let missed = missed_file_caps(1080, 1080, 350, 20.0, 1000, "gif", "gif");
        assert!(missed.contains(&"pixelFrames".into()));
    }

    #[test]
    fn stored_length_must_match() {
        assert!(meets_stored_caps(2048, Some(2048)));
        assert!(missed_stored_caps(2048, Some(2047)).contains(&"contentLengthMismatch".into()));
        assert!(missed_stored_caps(MAX_BYTES + 1, Some(MAX_BYTES + 1)).contains(&"bytes".into()));
    }

    #[test]
    fn gif_format_predicate() {
        assert!(is_gif_format("gif", "gif"));
        assert!(!is_gif_format("mp4", "h264"));
    }
}
