/// 2D Haar Discrete Wavelet Transform coefficients.
///
/// After a 1-level 2D Haar DWT, the image is decomposed into four subbands:
/// - `ll`: approximation (low-pass rows, low-pass columns)
/// - `lh`: detail (low-pass rows, high-pass columns) — horizontal edges
/// - `hl`: detail (high-pass rows, low-pass columns) — vertical edges
/// - `hh`: diagonal detail (high-pass rows, high-pass columns)
#[derive(Clone, Debug)]
pub struct DwtCoeffs {
    pub ll: Vec<Vec<f64>>,
    pub lh: Vec<Vec<f64>>,
    pub hl: Vec<Vec<f64>>,
    pub hh: Vec<Vec<f64>>,
    pub orig_rows: usize,
    pub orig_cols: usize,
}

/// Perform forward 1-level 2D Haar DWT.
///
/// Input dimensions are truncated to even numbers if necessary.
pub fn forward(image: &[Vec<f64>]) -> DwtCoeffs {
    let orig_rows = image.len();
    let orig_cols = if orig_rows > 0 { image[0].len() } else { 0 };
    let rows = orig_rows - (orig_rows % 2);
    let cols = orig_cols - (orig_cols % 2);
    let half_r = rows / 2;
    let half_c = cols / 2;

    // Step 1: Row-wise transform
    // For each row, pair adjacent pixels: low = (a+b)/2, high = (a-b)/2
    // Left half = row low-pass, right half = row high-pass
    let mut temp = vec![vec![0.0; cols]; rows];
    for r in 0..rows {
        for c in (0..cols).step_by(2) {
            let a = image[r][c];
            let b = image[r][c + 1];
            temp[r][c / 2] = (a + b) / 2.0;
            temp[r][half_c + c / 2] = (a - b) / 2.0;
        }
    }

    // Step 2: Column-wise transform on the result
    // Top half = column low-pass, bottom half = column high-pass
    let mut ll = vec![vec![0.0; half_c]; half_r];
    let mut lh = vec![vec![0.0; half_c]; half_r];
    let mut hl = vec![vec![0.0; half_c]; half_r];
    let mut hh = vec![vec![0.0; half_c]; half_r];

    for c in 0..half_c {
        for r in (0..rows).step_by(2) {
            let a = temp[r][c];
            let b = temp[r + 1][c];
            ll[r / 2][c] = (a + b) / 2.0;
            hl[r / 2][c] = (a - b) / 2.0;
        }
    }
    for c in 0..half_c {
        for r in (0..rows).step_by(2) {
            let a = temp[r][half_c + c];
            let b = temp[r + 1][half_c + c];
            lh[r / 2][c] = (a + b) / 2.0;
            hh[r / 2][c] = (a - b) / 2.0;
        }
    }

    DwtCoeffs {
        ll,
        lh,
        hl,
        hh,
        orig_rows,
        orig_cols,
    }
}

/// Perform inverse 1-level 2D Haar DWT.
///
/// Returns the reconstructed image matrix, cropped to the original dimensions.
pub fn inverse(coeffs: &DwtCoeffs) -> Vec<Vec<f64>> {
    let half_r = coeffs.ll.len();
    let half_c = coeffs.ll[0].len();
    let rows = half_r * 2;
    let cols = half_c * 2;

    // Reassemble into the interleaved layout:
    // top-left = LL, top-right = LH (row high-pass, col low-pass)
    // bottom-left = HL (row low-pass, col high-pass), bottom-right = HH

    // Step 1: Inverse column transform
    // For each column position, reconstruct pairs from low (top) and high (bottom)
    let mut temp = vec![vec![0.0; cols]; rows];

    // Left half columns (from LL and HL)
    #[allow(clippy::needless_range_loop)]
    for c in 0..half_c {
        for r in 0..half_r {
            let low = coeffs.ll[r][c];
            let high = coeffs.hl[r][c];
            temp[r * 2][c] = low + high;
            temp[r * 2 + 1][c] = low - high;
        }
    }
    // Right half columns (from LH and HH)
    for c in 0..half_c {
        for r in 0..half_r {
            let low = coeffs.lh[r][c];
            let high = coeffs.hh[r][c];
            temp[r * 2][half_c + c] = low + high;
            temp[r * 2 + 1][half_c + c] = low - high;
        }
    }

    // Step 2: Inverse row transform
    let mut result = vec![vec![0.0; cols]; rows];
    for r in 0..rows {
        for c in 0..half_c {
            let low = temp[r][c];
            let high = temp[r][half_c + c];
            result[r][c * 2] = low + high;
            result[r][c * 2 + 1] = low - high;
        }
    }

    // Crop to original dimensions
    let out_rows = coeffs.orig_rows.min(rows);
    let out_cols = coeffs.orig_cols.min(cols);
    result.truncate(out_rows);
    for row in &mut result {
        row.truncate(out_cols);
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dwt_round_trip() {
        let image = vec![
            vec![52.0, 55.0, 61.0, 66.0],
            vec![70.0, 61.0, 64.0, 73.0],
            vec![63.0, 59.0, 55.0, 90.0],
            vec![67.0, 85.0, 71.0, 64.0],
        ];

        let coeffs = forward(&image);
        let reconstructed = inverse(&coeffs);

        assert_eq!(image.len(), reconstructed.len());
        assert_eq!(image[0].len(), reconstructed[0].len());
        for r in 0..image.len() {
            for c in 0..image[0].len() {
                assert!(
                    (image[r][c] - reconstructed[r][c]).abs() < 1e-10,
                    "Mismatch at ({}, {}): {} vs {}",
                    r,
                    c,
                    image[r][c],
                    reconstructed[r][c]
                );
            }
        }
    }

    #[test]
    fn test_dwt_odd_dimensions() {
        let image = vec![
            vec![10.0, 20.0, 30.0, 40.0, 50.0],
            vec![60.0, 70.0, 80.0, 90.0, 100.0],
            vec![110.0, 120.0, 130.0, 140.0, 150.0],
        ];

        let coeffs = forward(&image);
        let reconstructed = inverse(&coeffs);

        // Should reconstruct the even-truncated portion
        assert_eq!(reconstructed.len(), 2);
        assert_eq!(reconstructed[0].len(), 4);
        for r in 0..2 {
            for c in 0..4 {
                assert!(
                    (image[r][c] - reconstructed[r][c]).abs() < 1e-10,
                    "Mismatch at ({}, {})",
                    r,
                    c
                );
            }
        }
    }

    #[test]
    fn test_dwt_subband_sizes() {
        let image = vec![vec![0.0; 8]; 6];
        let coeffs = forward(&image);
        assert_eq!(coeffs.ll.len(), 3);
        assert_eq!(coeffs.ll[0].len(), 4);
        assert_eq!(coeffs.lh.len(), 3);
        assert_eq!(coeffs.lh[0].len(), 4);
        assert_eq!(coeffs.hl.len(), 3);
        assert_eq!(coeffs.hl[0].len(), 4);
        assert_eq!(coeffs.hh.len(), 3);
        assert_eq!(coeffs.hh[0].len(), 4);
    }
}
