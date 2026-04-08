//! Feature point detection and patch extraction for cropping-resistant watermarking.
//!
//! Uses oriented FAST corners (from `imageproc`) as Local Feature Regions (LFRs).
//! Each keypoint provides a position and orientation angle, allowing patches to be
//! extracted and rotated to a canonical orientation for embedding/extraction.

use image::{GrayImage, Luma};
use imageproc::corners::{oriented_fast, OrientedFastCorner};
use imageproc::geometric_transformations::{rotate_about_center, Interpolation};

/// Size of the square patch extracted around each keypoint.
pub const PATCH_SIZE: usize = 64;

/// Half the patch size — the radius from keypoint center to patch edge.
const HALF_PATCH: u32 = PATCH_SIZE as u32 / 2;

/// Minimum distance from image edge for a keypoint to be usable.
/// Must be at least HALF_PATCH to extract a full patch, plus margin for rotation.
const EDGE_MARGIN: u32 = HALF_PATCH + 8;

/// A detected feature point with its position, orientation, and response strength.
#[derive(Debug, Clone, Copy)]
pub struct FeaturePoint {
    pub x: u32,
    pub y: u32,
    pub orientation: f32,
    pub score: f32,
}

/// Detect oriented FAST corners in a grayscale image.
///
/// Returns up to `max_keypoints` feature points, sorted by response strength
/// (strongest first). Keypoints too close to the image edge are excluded.
pub fn detect_keypoints(gray: &GrayImage, max_keypoints: usize) -> Vec<FeaturePoint> {
    let (width, height) = gray.dimensions();

    // Need enough room for patch extraction + rotation margin
    if width < EDGE_MARGIN * 2 + 1 || height < EDGE_MARGIN * 2 + 1 {
        return Vec::new();
    }

    // Detect oriented FAST corners
    // edge_radius=EDGE_MARGIN ensures no corners near edges
    // target_num_corners guides adaptive thresholding
    let corners: Vec<OrientedFastCorner> = oriented_fast(
        gray,
        None,              // auto threshold
        max_keypoints * 2, // request more than needed, we'll filter
        EDGE_MARGIN,       // edge exclusion radius
        Some(42),          // deterministic seed for reproducibility
    );

    // Convert to FeaturePoint and sort by score (strongest first)
    let mut points: Vec<FeaturePoint> = corners
        .into_iter()
        .map(|c| FeaturePoint {
            x: c.corner.x,
            y: c.corner.y,
            orientation: c.orientation,
            score: c.corner.score,
        })
        .collect();

    points.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    points.truncate(max_keypoints);
    points
}

/// Extract a PATCH_SIZE × PATCH_SIZE patch from a grayscale image centered on a keypoint,
/// rotated to canonical orientation (orientation angle removed).
///
/// Returns the patch as a Vec<f64> in row-major order.
pub fn extract_normalized_patch(gray: &GrayImage, kp: &FeaturePoint) -> Vec<f64> {
    // Extract a larger region to account for rotation (diagonal of patch)
    let extract_radius = (HALF_PATCH as f64 * std::f64::consts::SQRT_2).ceil() as u32 + 2;
    let extract_size = extract_radius * 2 + 1;

    // Extract the region centered on keypoint
    let mut region = GrayImage::new(extract_size, extract_size);
    let cx = kp.x as i64;
    let cy = kp.y as i64;

    for ry in 0..extract_size {
        for rx in 0..extract_size {
            let sx = cx - extract_radius as i64 + rx as i64;
            let sy = cy - extract_radius as i64 + ry as i64;
            if sx >= 0 && sy >= 0 && (sx as u32) < gray.width() && (sy as u32) < gray.height() {
                region.put_pixel(rx, ry, *gray.get_pixel(sx as u32, sy as u32));
            }
        }
    }

    // Rotate to cancel the keypoint orientation → canonical angle
    let rotated = rotate_about_center(
        &region,
        -kp.orientation,
        Interpolation::Bilinear,
        Luma([0u8]),
    );

    // Crop the center PATCH_SIZE × PATCH_SIZE from the rotated region
    let rot_cx = rotated.width() / 2;
    let rot_cy = rotated.height() / 2;
    let mut patch = vec![0.0f64; PATCH_SIZE * PATCH_SIZE];

    for py in 0..PATCH_SIZE {
        for px in 0..PATCH_SIZE {
            let sx = rot_cx as i64 - HALF_PATCH as i64 + px as i64;
            let sy = rot_cy as i64 - HALF_PATCH as i64 + py as i64;
            if sx >= 0 && sy >= 0 && (sx as u32) < rotated.width() && (sy as u32) < rotated.height()
            {
                patch[py * PATCH_SIZE + px] = rotated.get_pixel(sx as u32, sy as u32).0[0] as f64;
            }
        }
    }

    patch
}

/// Write a modified patch back to the image at the keypoint location,
/// reversing the canonical rotation. `mask` is an optional Gaussian blending
/// mask (same size as patch) with values 0.0-1.0.
pub fn write_patch_back(channel: &mut [Vec<f64>], kp: &FeaturePoint, patch: &[f64], mask: &[f64]) {
    let height = channel.len() as u32;
    let width = channel[0].len() as u32;

    // Build a GrayImage from the patch for rotation
    let mut patch_img = GrayImage::new(PATCH_SIZE as u32, PATCH_SIZE as u32);
    for py in 0..PATCH_SIZE {
        for px in 0..PATCH_SIZE {
            let v = patch[py * PATCH_SIZE + px].round().clamp(0.0, 255.0) as u8;
            patch_img.put_pixel(px as u32, py as u32, Luma([v]));
        }
    }

    // Rotate back by +orientation (undo the canonical rotation)
    let rotated = rotate_about_center(
        &patch_img,
        kp.orientation,
        Interpolation::Bilinear,
        Luma([0u8]),
    );

    // Also rotate the mask
    let mut mask_img = GrayImage::new(PATCH_SIZE as u32, PATCH_SIZE as u32);
    for py in 0..PATCH_SIZE {
        for px in 0..PATCH_SIZE {
            let v = (mask[py * PATCH_SIZE + px] * 255.0)
                .round()
                .clamp(0.0, 255.0) as u8;
            mask_img.put_pixel(px as u32, py as u32, Luma([v]));
        }
    }
    let rotated_mask = rotate_about_center(
        &mask_img,
        kp.orientation,
        Interpolation::Bilinear,
        Luma([0u8]),
    );

    // Blend rotated patch back into the channel using the rotated mask
    let rot_cx = rotated.width() / 2;
    let rot_cy = rotated.height() / 2;

    for ry in 0..rotated.height() {
        for rx in 0..rotated.width() {
            let dx = rx as i64 - rot_cx as i64;
            let dy = ry as i64 - rot_cy as i64;
            let tx = kp.x as i64 + dx;
            let ty = kp.y as i64 + dy;

            if tx >= 0 && ty >= 0 && (tx as u32) < width && (ty as u32) < height {
                let alpha = rotated_mask.get_pixel(rx, ry).0[0] as f64 / 255.0;
                if alpha > 0.001 {
                    let old_val = channel[ty as usize][tx as usize];
                    let new_val = rotated.get_pixel(rx, ry).0[0] as f64;
                    channel[ty as usize][tx as usize] = old_val * (1.0 - alpha) + new_val * alpha;
                }
            }
        }
    }
}

/// Generate a circular Gaussian blending mask for a PATCH_SIZE × PATCH_SIZE patch.
///
/// Center pixels have weight 1.0, edges fall off smoothly to 0.0.
/// This prevents visible seams when blending watermarked patches back.
pub fn gaussian_blend_mask() -> Vec<f64> {
    let mut mask = vec![0.0f64; PATCH_SIZE * PATCH_SIZE];
    let center = PATCH_SIZE as f64 / 2.0;
    let sigma = PATCH_SIZE as f64 / 4.0; // Gaussian sigma

    for py in 0..PATCH_SIZE {
        for px in 0..PATCH_SIZE {
            let dx = px as f64 + 0.5 - center;
            let dy = py as f64 + 0.5 - center;
            let dist_sq = dx * dx + dy * dy;
            let weight = (-dist_sq / (2.0 * sigma * sigma)).exp();
            mask[py * PATCH_SIZE + px] = weight;
        }
    }
    mask
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gaussian_mask_properties() {
        let mask = gaussian_blend_mask();
        assert_eq!(mask.len(), PATCH_SIZE * PATCH_SIZE);

        // Center should be close to 1.0
        let center_idx = (PATCH_SIZE / 2) * PATCH_SIZE + PATCH_SIZE / 2;
        assert!(mask[center_idx] > 0.95, "Center weight should be ~1.0");

        // Corners should be much lower
        assert!(mask[0] < 0.2, "Corner weight should be low");

        // Symmetric
        let last = PATCH_SIZE * PATCH_SIZE - 1;
        assert!(
            (mask[0] - mask[last]).abs() < 1e-10,
            "Mask should be symmetric"
        );
    }

    #[test]
    fn test_detect_keypoints_small_image() {
        // Image too small for any keypoints
        let img = GrayImage::new(32, 32);
        let kps = detect_keypoints(&img, 100);
        assert!(kps.is_empty());
    }

    #[test]
    fn test_detect_keypoints_gradient() {
        // Create an image with clear corners (a white rectangle on dark background)
        let mut img = GrayImage::from_pixel(256, 256, Luma([30u8]));
        for y in 80..180 {
            for x in 80..180 {
                img.put_pixel(x, y, Luma([200u8]));
            }
        }

        let kps = detect_keypoints(&img, 50);
        // Should detect corners of the rectangle
        assert!(!kps.is_empty(), "Should detect corners in rectangle image");
        // Keypoints should be sorted by score
        for i in 1..kps.len() {
            assert!(
                kps[i - 1].score >= kps[i].score,
                "Should be sorted by score"
            );
        }
    }

    #[test]
    fn test_extract_normalized_patch_dimensions() {
        let mut img = GrayImage::from_pixel(256, 256, Luma([128u8]));
        // Add some texture
        for y in 0..256u32 {
            for x in 0..256u32 {
                img.put_pixel(x, y, Luma([((x * 7 + y * 13) % 256) as u8]));
            }
        }

        let kp = FeaturePoint {
            x: 128,
            y: 128,
            orientation: 0.0,
            score: 100.0,
        };

        let patch = extract_normalized_patch(&img, &kp);
        assert_eq!(patch.len(), PATCH_SIZE * PATCH_SIZE);
    }

    #[test]
    fn test_keypoints_deterministic() {
        let mut img = GrayImage::from_pixel(256, 256, Luma([30u8]));
        for y in 80..180 {
            for x in 80..180 {
                img.put_pixel(x, y, Luma([200u8]));
            }
        }

        let kps1 = detect_keypoints(&img, 50);
        let kps2 = detect_keypoints(&img, 50);
        assert_eq!(kps1.len(), kps2.len());
        for (a, b) in kps1.iter().zip(kps2.iter()) {
            assert_eq!(a.x, b.x);
            assert_eq!(a.y, b.y);
        }
    }
}
