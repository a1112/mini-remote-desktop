use mrd_pipeline_core::{CapturedFrame, FrameCapture, FramePixelFormat, PipelineError};
use scrap::{Capturer, Display};
use std::{
    io::ErrorKind,
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

pub struct DxgiDesktopCapture {
    capturer: Capturer,
    width: usize,
    height: usize,
}

impl DxgiDesktopCapture {
    pub fn new_primary() -> Result<Self, PipelineError> {
        let display = Display::primary()
            .map_err(|error| PipelineError::message(format!("open primary display failed: {error}")))?;
        Self::new(display)
    }

    pub fn new(display: Display) -> Result<Self, PipelineError> {
        let width = display.width();
        let height = display.height();
        let capturer = Capturer::new(display)
            .map_err(|error| PipelineError::message(format!("create dxgi capturer failed: {error}")))?;

        Ok(Self {
            capturer,
            width,
            height,
        })
    }

    pub fn width(&self) -> usize {
        self.width
    }

    pub fn height(&self) -> usize {
        self.height
    }
}

impl FrameCapture for DxgiDesktopCapture {
    fn capture_frame(&mut self) -> Result<CapturedFrame, PipelineError> {
        loop {
            match self.capturer.frame() {
                Ok(frame) => {
                    let packed = repack_bgra(frame.as_ref(), self.width, self.height)?;
                    let timestamp_us = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .map_err(|error| PipelineError::message(format!("system time failed: {error}")))?
                        .as_micros() as u64;

                    return Ok(CapturedFrame {
                        width: self.width,
                        height: self.height,
                        pixel_format: FramePixelFormat::Bgra32,
                        timestamp_us,
                        data: packed,
                    });
                }
                Err(error) if error.kind() == ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(5));
                }
                Err(error) => {
                    return Err(PipelineError::message(format!("capture frame failed: {error}")));
                }
            }
        }
    }
}

fn repack_bgra(frame: &[u8], width: usize, height: usize) -> Result<Vec<u8>, PipelineError> {
    let stride = frame
        .len()
        .checked_div(height.max(1))
        .ok_or_else(|| PipelineError::message("invalid captured frame height"))?;
    let row_bytes = width
        .checked_mul(4)
        .ok_or_else(|| PipelineError::message("captured frame width overflow"))?;

    if stride < row_bytes || frame.len() < stride * height {
        return Err(PipelineError::message("invalid captured frame stride"));
    }

    let mut packed = Vec::with_capacity(row_bytes * height);
    for row in 0..height {
        let start = row * stride;
        packed.extend_from_slice(&frame[start..start + row_bytes]);
    }
    Ok(packed)
}

#[cfg(test)]
mod tests {
    use super::repack_bgra;

    #[test]
    fn repack_bgra_strips_padding_stride() {
        let frame = vec![
            1, 2, 3, 4, 5, 6, 7, 8, 0, 0, 0, 0,
            9, 10, 11, 12, 13, 14, 15, 16, 0, 0, 0, 0,
        ];

        let packed = repack_bgra(&frame, 2, 2).expect("packed frame");

        assert_eq!(packed, vec![1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16]);
    }
}
