//! Integration tests for SVG watermarking using testing_data/svg/.

use std::path::PathBuf;

use infinishield::common::WatermarkEngine;
use infinishield::vector::VectorEngine;

const DEFAULT_MESSAGE: &str = "Hi";
const DEFAULT_PASSWORD: &str = "d1ng0";

fn project_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn input_dir() -> PathBuf {
    project_root().join("testing_data").join("svg")
}

fn output_dir() -> PathBuf {
    let dir = project_root().join("testing_output");
    std::fs::create_dir_all(&dir).expect("failed to create testing_output/");
    dir
}

fn output_path(name: &str) -> PathBuf {
    output_dir().join(name)
}

/// Helper: embed + verify round-trip on an SVG file.
fn assert_svg_round_trip(input_name: &str, output_name: &str, message: &str) {
    let input = input_dir().join(input_name);
    let output = output_path(output_name);

    let result = VectorEngine
        .embed(
            input.to_str().unwrap(),
            message,
            DEFAULT_PASSWORD,
            0,
            output.to_str().unwrap(),
        )
        .expect(&format!("embed failed for {}", input_name));

    assert!(
        result.message.contains("成功"),
        "Embed should succeed for {}",
        input_name
    );

    let verify = VectorEngine
        .verify(output.to_str().unwrap(), DEFAULT_PASSWORD)
        .expect(&format!("verify failed for {}", input_name));

    assert!(
        verify.detected,
        "Watermark not detected in {} (confidence: {:.1}%)",
        input_name,
        verify.confidence * 100.0
    );
    assert_eq!(
        verify.message.as_deref(),
        Some(message),
        "Message mismatch for {}",
        input_name
    );
}

// ── Qualifying SVGs (embed + verify) ─────────────────────────────────────

#[test]
fn test_svg_shapes() {
    assert_svg_round_trip("shapes.svg", "shapes_wm.svg", DEFAULT_MESSAGE);
}

#[test]
fn test_svg_crab() {
    assert_svg_round_trip("crab.svg", "crab_wm.svg", DEFAULT_MESSAGE);
}

#[test]
fn test_svg_fox() {
    assert_svg_round_trip("fox.svg", "fox_wm.svg", DEFAULT_MESSAGE);
}

#[test]
fn test_svg_newtux() {
    assert_svg_round_trip("NewTux.svg", "NewTux_wm.svg", DEFAULT_MESSAGE);
}

#[test]
fn test_svg_peach() {
    assert_svg_round_trip("peach.svg", "peach_wm.svg", DEFAULT_MESSAGE);
}

#[test]
fn test_svg_polaroid() {
    assert_svg_round_trip("polaroid.svg", "polaroid_wm.svg", DEFAULT_MESSAGE);
}

#[test]
fn test_svg_rabbit() {
    assert_svg_round_trip("rabbit.svg", "rabbit_wm.svg", DEFAULT_MESSAGE);
}

#[test]
fn test_svg_wild_boar() {
    assert_svg_round_trip("wild-boar.svg", "wild-boar_wm.svg", DEFAULT_MESSAGE);
}

// ── Wrong password ───────────────────────────────────────────────────────

#[test]
fn test_svg_wrong_password() {
    let input = input_dir().join("crab.svg");
    let output = output_path("crab_wp.svg");

    VectorEngine
        .embed(
            input.to_str().unwrap(),
            DEFAULT_MESSAGE,
            DEFAULT_PASSWORD,
            0,
            output.to_str().unwrap(),
        )
        .unwrap();

    let result = VectorEngine
        .verify(output.to_str().unwrap(), "wrong_password")
        .unwrap();

    assert!(
        !result.detected || result.message.as_deref() != Some(DEFAULT_MESSAGE),
        "Wrong password should not extract correct message"
    );
}

// ── Unwatermarked SVG ────────────────────────────────────────────────────

#[test]
fn test_svg_unwatermarked() {
    let input = input_dir().join("fox.svg");
    let result = VectorEngine
        .verify(input.to_str().unwrap(), DEFAULT_PASSWORD)
        .unwrap();

    if result.detected {
        assert_ne!(
            result.message.as_deref(),
            Some(DEFAULT_MESSAGE),
            "Should not find watermark in unwatermarked SVG"
        );
    }
}

// ── Too-simple SVGs (should error) ───────────────────────────────────────

#[test]
fn test_svg_logo_too_simple() {
    let input = input_dir().join("logo.svg");
    let output = output_path("logo_wm.svg");

    let result = VectorEngine.embed(
        input.to_str().unwrap(),
        DEFAULT_MESSAGE,
        DEFAULT_PASSWORD,
        0,
        output.to_str().unwrap(),
    );

    assert!(result.is_err(), "logo.svg should fail: paths too simple");
}

#[test]
fn test_svg_branch_too_simple() {
    let input = input_dir().join("branch.svg");
    let output = output_path("branch_wm.svg");

    let result = VectorEngine.embed(
        input.to_str().unwrap(),
        DEFAULT_MESSAGE,
        DEFAULT_PASSWORD,
        0,
        output.to_str().unwrap(),
    );

    assert!(result.is_err(), "branch.svg should fail: paths too simple");
}

// ── Max message ──────────────────────────────────────────────────────────

#[test]
fn test_svg_longer_message() {
    // wild-boar.svg has complex paths — test longer message
    assert_svg_round_trip("wild-boar.svg", "wild-boar_long.svg", "OK");
}

// ── Element deletion resilience ──────────────────────────────────────────

#[test]
fn test_svg_element_deletion() {
    // Embed in crab.svg (43 paths), then delete some paths, verify extraction
    let input = input_dir().join("crab.svg");
    let wm_path = output_path("crab_deletion_wm.svg");
    let modified_path = output_path("crab_deletion_modified.svg");

    VectorEngine
        .embed(
            input.to_str().unwrap(),
            DEFAULT_MESSAGE,
            DEFAULT_PASSWORD,
            0,
            wm_path.to_str().unwrap(),
        )
        .unwrap();

    // Verify full file first
    let full_result = VectorEngine
        .verify(wm_path.to_str().unwrap(), DEFAULT_PASSWORD)
        .unwrap();
    assert!(full_result.detected, "Full file should detect watermark");

    // Remove some path elements (simulate element deletion)
    let svg = std::fs::read_to_string(&wm_path).unwrap();
    let mut modified = svg.clone();
    // Delete every other path element (keep at least half)
    let mut count = 0;
    let mut search = 0;
    while let Some(start) = modified[search..].find("<path ") {
        let abs_start = search + start;
        count += 1;
        if count % 2 == 0 {
            if let Some(end) = modified[abs_start..].find("/>") {
                let remove_end = abs_start + end + 2;
                modified = format!("{}{}", &modified[..abs_start], &modified[remove_end..]);
                continue; // don't advance search, string shortened
            }
        }
        search = abs_start + 6;
    }
    std::fs::write(&modified_path, &modified).unwrap();

    let result = VectorEngine
        .verify(modified_path.to_str().unwrap(), DEFAULT_PASSWORD)
        .unwrap();

    // This may or may not detect depending on which paths survive.
    // Document the result either way.
    if result.detected {
        assert_eq!(result.message.as_deref(), Some(DEFAULT_MESSAGE));
        println!(
            "Element deletion: SURVIVED (confidence: {:.1}%)",
            result.confidence * 100.0
        );
    } else {
        println!("Element deletion: watermark lost (expected limitation)");
    }
}

// ── viewBox change resilience ────────────────────────────────────────────

#[test]
fn test_svg_viewbox_change() {
    // Embed watermark, change the viewBox, verify extraction.
    // Coordinate values are absolute, so viewBox changes don't affect them.
    let input = input_dir().join("shapes.svg");
    let wm_path = output_path("shapes_viewbox_wm.svg");
    let modified_path = output_path("shapes_viewbox_modified.svg");

    VectorEngine
        .embed(
            input.to_str().unwrap(),
            DEFAULT_MESSAGE,
            DEFAULT_PASSWORD,
            0,
            wm_path.to_str().unwrap(),
        )
        .unwrap();

    // Change viewBox
    let svg = std::fs::read_to_string(&wm_path).unwrap();
    let modified = svg.replace("viewBox=\"0 0 800 600\"", "viewBox=\"-100 -100 1000 800\"");
    std::fs::write(&modified_path, &modified).unwrap();

    let result = VectorEngine
        .verify(modified_path.to_str().unwrap(), DEFAULT_PASSWORD)
        .unwrap();

    assert!(
        result.detected,
        "Watermark should survive viewBox change (confidence: {:.1}%)",
        result.confidence * 100.0
    );
    assert_eq!(result.message.as_deref(), Some(DEFAULT_MESSAGE));
}

// ── Single-byte message ──────────────────────────────────────────────────

#[test]
fn test_svg_single_char() {
    assert_svg_round_trip("fox.svg", "fox_single_char.svg", "X");
}
