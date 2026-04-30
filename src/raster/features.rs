//! Feature point detection and patch extraction for cropping-resistant watermarking.
//!
//! Uses oriented FAST corners (from `imageproc`) as Local Feature Regions (LFRs).
//! Each keypoint provides a position and orientation angle, allowing patches to be
//! extracted and rotated to a canonical orientation for embedding/extraction.

use image::{GrayImage, Luma};
use imageproc::corners::{oriented_fast, OrientedFastCorner};

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
/// Returns the patch as a `Vec<f64>` in row-major order.

/// Write a modified patch back to the image at the keypoint location,
/// reversing the canonical rotation. `mask` is an optional Gaussian blending
/// mask (same size as patch) with values 0.0-1.0.

/// Generate a circular Gaussian blending mask for a PATCH_SIZE × PATCH_SIZE patch.
///
/// Center pixels have weight 1.0, edges fall off smoothly to 0.0.
/// This prevents visible seams when blending watermarked patches back.

#[cfg(test)]
mod tests {
    use super::*;

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
