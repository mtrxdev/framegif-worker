use crate::caps::{is_gif_format, missed_file_caps, missed_stored_caps, pixel_frames};
use crate::types::{EncodeAttempt, EncodeError, MAX_BYTES, Measure, ObjectLocator};

#[derive(Debug, Clone)]
pub struct Gif {
    bytes: u64,
    width: u32,
    height: u32,
    frames: u32,
    fps: f64,
    quality: u8,
    locator: ObjectLocator,
    prior_misses: Vec<EncodeAttempt>,
}

impl Gif {
    pub const MAX_BYTES: u64 = MAX_BYTES;

    pub fn certify(measure: &Measure, content_length: Option<u64>) -> Result<Self, EncodeError> {
        if pixel_frames(measure.width, measure.height, measure.frames) != measure.pixels {
            return Err(EncodeError::Head {
                local: measure.bytes,
                stored: content_length,
            });
        }
        if !missed_file_caps(
            measure.width,
            measure.height,
            measure.frames,
            measure.fps,
            measure.bytes,
            &measure.format_name,
            &measure.codec_name,
        )
        .is_empty()
            || !is_gif_format(&measure.format_name, &measure.codec_name)
            || !missed_stored_caps(measure.bytes, content_length).is_empty()
        {
            return Err(EncodeError::Head {
                local: measure.bytes,
                stored: content_length,
            });
        }
        Ok(Self {
            bytes: measure.bytes,
            width: measure.width,
            height: measure.height,
            frames: measure.frames,
            fps: measure.fps,
            quality: measure.quality,
            locator: ObjectLocator::default(),
            prior_misses: Vec::new(),
        })
    }

    pub(crate) fn bind(mut self, locator: ObjectLocator, prior_misses: Vec<EncodeAttempt>) -> Self {
        self.locator = locator;
        self.prior_misses = prior_misses;
        self
    }

    pub fn locator(&self) -> &ObjectLocator {
        &self.locator
    }

    pub fn prior_misses(&self) -> &[EncodeAttempt] {
        &self.prior_misses
    }

    pub fn bytes(&self) -> u64 {
        self.bytes
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn frames(&self) -> u32 {
        self.frames
    }

    pub fn pixels(&self) -> u64 {
        pixel_frames(self.width, self.height, self.frames)
    }

    pub fn fps(&self) -> f64 {
        self.fps
    }

    pub fn quality(&self) -> u8 {
        self.quality
    }

    pub const fn max_bytes(&self) -> u64 {
        Self::MAX_BYTES
    }

    pub const fn within_caps(&self) -> bool {
        true
    }

    pub fn measure(&self) -> Measure {
        Measure {
            width: self.width,
            height: self.height,
            frames: self.frames,
            pixels: self.pixels(),
            fps: self.fps,
            quality: self.quality,
            bytes: self.bytes,
            format_name: "gif".into(),
            codec_name: "gif".into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gif_certify_rejects_pixel_overflow() {
        let measure = Measure {
            width: 1080,
            height: 1080,
            frames: 350,
            pixels: pixel_frames(1080, 1080, 350),
            fps: 20.0,
            quality: 90,
            bytes: 2048,
            format_name: "gif".into(),
            codec_name: "gif".into(),
        };
        assert!(Gif::certify(&measure, Some(2048)).is_err());
    }

    #[test]
    fn gif_within_caps_is_always_true() {
        let measure = Measure {
            width: 100,
            height: 80,
            frames: 10,
            pixels: pixel_frames(100, 80, 10),
            fps: 10.0,
            quality: 90,
            bytes: 2048,
            format_name: "gif".into(),
            codec_name: "gif".into(),
        };
        let gif = Gif::certify(&measure, Some(2048)).unwrap();
        assert!(gif.within_caps());
        assert_eq!(gif.max_bytes(), 15_728_640);
        assert_eq!(gif.pixels(), 80_000);
    }
}
