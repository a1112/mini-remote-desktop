/// RGB24 to BGRA conversion utilities

/// Converts RGB24 to BGRA32 using optimized methods
///
/// # Arguments
/// * `src` - RGB24 source data (width * height * 3 bytes)
/// * `dst` - BGRA32 destination buffer (width * height * 4 bytes)
/// * `width` - Image width in pixels
/// * `height` - Image height in pixels
pub fn rgb24_to_bgra(src: &[u8], dst: &mut [u8], width: usize, height: usize) {
    let total_pixels = width * height;
    assert_eq!(src.len(), total_pixels * 3);
    assert_eq!(dst.len(), total_pixels * 4);

    #[cfg(target_arch = "x86_64")]
    unsafe {
        if is_x86_feature_detected!("sse2") {
            rgb24_to_bgra_sse2_impl(src, dst, total_pixels);
            return;
        }
    }

    // Scalar fallback
    rgb24_to_bgra_scalar(src, dst, total_pixels);
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "sse2")]
unsafe fn rgb24_to_bgra_sse2_impl(src: &[u8], dst: &mut [u8], pixels: usize) {
    let src_ptr = src.as_ptr();
    let dst_ptr = dst.as_mut_ptr();

    // Process 2 pixels at a time (6 bytes -> 8 bytes fits in 64-bit register)
    let chunks = pixels / 2;
    let mut i = 0;

    for _ in 0..chunks {
        let src_offset = i * 3;
        let dst_offset = i * 4;

        // Load 6 bytes (2 RGB pixels)
        let rgb = *(src_ptr.add(src_offset) as *const [u8; 6]);

        // Convert to BGRA
        // Pixel 1: R1 G1 B1 -> B1 G1 R1 A
        // Pixel 2: R2 G2 B2 -> B2 G2 R2 A
        let bgra = [
            rgb[2], rgb[1], rgb[0], 0xFF,  // Pixel 1
            rgb[5], rgb[4], rgb[3], 0xFF,  // Pixel 2
        ];

        *(dst_ptr.add(dst_offset) as *mut [u8; 8]) = bgra;

        i += 2;
    }

    // Handle remaining pixel
    if pixels % 2 == 1 {
        let offset = i * 3;
        let r = *src_ptr.add(offset);
        let g = *src_ptr.add(offset + 1);
        let b = *src_ptr.add(offset + 2);

        let dst_offset = i * 4;
        *dst_ptr.add(dst_offset) = b;
        *dst_ptr.add(dst_offset + 1) = g;
        *dst_ptr.add(dst_offset + 2) = r;
        *dst_ptr.add(dst_offset + 3) = 0xFF;
    }
}

fn rgb24_to_bgra_scalar(src: &[u8], dst: &mut [u8], pixels: usize) {
    let mut src_idx = 0;
    let mut dst_idx = 0;

    for _ in 0..pixels {
        let r = src[src_idx];
        let g = src[src_idx + 1];
        let b = src[src_idx + 2];

        dst[dst_idx] = b;
        dst[dst_idx + 1] = g;
        dst[dst_idx + 2] = r;
        dst[dst_idx + 3] = 0xFF;

        src_idx += 3;
        dst_idx += 4;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rgb24_to_bgra_correctness() {
        let src = vec![
            255, 0, 0,    // Red
            0, 255, 0,    // Green
            0, 0, 255,    // Blue
            255, 255, 0,  // Yellow
        ];
        let mut dst = vec![0_u8; 16];

        rgb24_to_bgra(&src, &mut dst, 4, 1);

        // Red -> BGRA: [0, 0, 255, 255]
        assert_eq!(dst[0..4], [0, 0, 255, 255]);
        // Green -> BGRA: [0, 255, 0, 255]
        assert_eq!(dst[4..8], [0, 255, 0, 255]);
        // Blue -> BGRA: [255, 0, 0, 255]
        assert_eq!(dst[8..12], [255, 0, 0, 255]);
        // Yellow -> BGRA: [0, 255, 255, 255]
        assert_eq!(dst[12..16], [0, 255, 255, 255]);
    }

    #[test]
    fn test_rgb24_to_bgra_odd_pixels() {
        let src = vec![100, 150, 200];  // 1 pixel
        let mut dst = vec![0_u8; 4];

        rgb24_to_bgra(&src, &mut dst, 1, 1);

        assert_eq!(dst, [200, 150, 100, 255]);
    }

    #[test]
    fn test_rgb24_to_bgra_720p() {
        let width = 1280;
        let height = 720;
        let pixels = width * height;

        let src: Vec<u8> = (0..pixels * 3).map(|i| (i % 256) as u8).collect();
        let mut dst = vec![0_u8; pixels * 4];

        rgb24_to_bgra(&src, &mut dst, width, height);

        // Verify first pixel: [0, 1, 2] -> [2, 1, 0, 255]
        assert_eq!(dst[0..4], [2, 1, 0, 255]);
    }
}
