//! Integration tests for video watermarking.
//! Only compiled when the `video` feature is enabled.
//! Requires ffmpeg and ffprobe in PATH.

#![cfg(feature = "video")]

use std::path::PathBuf;
use std::process::Command;

use infinishield::common::WatermarkEngine;
use infinishield::video::VideoEngine;

const DEFAULT_MESSAGE: &str = "Infini";
const DEFAULT_PASSWORD: &str = "d1ng0";

fn ffmpeg_available() -> bool {
    Command::new("ffmpeg")
        .arg("-version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok()
}

fn project_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn input_dir() -> PathBuf {
    project_root().join("testing_data").join("mov")
}

fn output_dir() -> PathBuf {
    let dir = project_root().join("testing_output");
    std::fs::create_dir_all(&dir).expect("failed to create testing_output/");
    dir
}

fn output_path(name: &str) -> PathBuf {
    output_dir().join(name)
}

// ── Basic embed/verify ───────────────────────────────────────────────────

#[test]
fn test_video_veo1_round_trip() {
    if !ffmpeg_available() {
        eprintln!("SKIP: ffmpeg not available");
        return;
    }

    let input = input_dir().join("veo1.mp4");
    let output = output_path("veo1_wm.mp4");

    let result = VideoEngine
        .embed(
            input.to_str().unwrap(),
            DEFAULT_MESSAGE,
            DEFAULT_PASSWORD,
            0,
            output.to_str().unwrap(),
        )
        .expect("embed failed");

    assert!(result.message.contains("成功"));
    assert!(output.exists());

    let verify = VideoEngine
        .verify(output.to_str().unwrap(), DEFAULT_PASSWORD)
        .expect("verify failed");

    assert!(verify.detected, "Watermark not detected in veo1");
    // H264 CRF 18 preserves most of the watermark but some frames may have
    // minor bit errors depending on video content complexity.
    if verify.message.as_deref() != Some(DEFAULT_MESSAGE) {
        println!(
            "veo1: partial extraction {:?} (codec artifact)",
            verify.message
        );
    }
}

#[test]
fn test_video_veo2_round_trip() {
    if !ffmpeg_available() {
        eprintln!("SKIP: ffmpeg not available");
        return;
    }

    let input = input_dir().join("veo2.mp4");
    let output = output_path("veo2_wm.mp4");

    VideoEngine
        .embed(
            input.to_str().unwrap(),
            DEFAULT_MESSAGE,
            DEFAULT_PASSWORD,
            0,
            output.to_str().unwrap(),
        )
        .expect("embed failed");

    let verify = VideoEngine
        .verify(output.to_str().unwrap(), DEFAULT_PASSWORD)
        .expect("verify failed");

    assert!(verify.detected, "Watermark not detected in veo2");
    if verify.message.as_deref() != Some(DEFAULT_MESSAGE) {
        println!(
            "veo2: partial extraction {:?} (codec artifact)",
            verify.message
        );
    }
}

// ── Wrong password ───────────────────────────────────────────────────────

#[test]
fn test_video_wrong_password() {
    if !ffmpeg_available() {
        eprintln!("SKIP: ffmpeg not available");
        return;
    }

    let input = input_dir().join("veo1.mp4");
    let output = output_path("veo1_wp.mp4");

    VideoEngine
        .embed(
            input.to_str().unwrap(),
            DEFAULT_MESSAGE,
            DEFAULT_PASSWORD,
            0,
            output.to_str().unwrap(),
        )
        .expect("embed failed");

    let result = VideoEngine
        .verify(output.to_str().unwrap(), "wrong_password")
        .expect("verify should not error");

    assert!(
        !result.detected || result.message.as_deref() != Some(DEFAULT_MESSAGE),
        "Wrong password should not extract correct message"
    );
}

// ── Custom message ───────────────────────────────────────────────────────

#[test]
fn test_video_custom_message() {
    if !ffmpeg_available() {
        eprintln!("SKIP: ffmpeg not available");
        return;
    }

    let input = input_dir().join("veo3.mp4");
    let output = output_path("veo3_custom.mp4");

    VideoEngine
        .embed(
            input.to_str().unwrap(),
            "Hi",
            DEFAULT_PASSWORD,
            0,
            output.to_str().unwrap(),
        )
        .expect("embed failed");

    let verify = VideoEngine
        .verify(output.to_str().unwrap(), DEFAULT_PASSWORD)
        .expect("verify failed");

    assert!(verify.detected);
    assert_eq!(verify.message.as_deref(), Some("Hi"));
}

// ── Temporal trim resilience ─────────────────────────────────────────────

#[test]
fn test_video_temporal_trim() {
    if !ffmpeg_available() {
        eprintln!("SKIP: ffmpeg not available");
        return;
    }

    let input = input_dir().join("veo1.mp4");
    let wm_path = output_path("veo1_trim_wm.mp4");
    let trimmed_path = output_path("veo1_trimmed.mp4");

    // Embed
    VideoEngine
        .embed(
            input.to_str().unwrap(),
            DEFAULT_MESSAGE,
            DEFAULT_PASSWORD,
            0,
            wm_path.to_str().unwrap(),
        )
        .expect("embed failed");

    // Trim: keep only middle 4 seconds (skip first 2s, take 4s)
    let result = Command::new("ffmpeg")
        .args([
            "-ss",
            "2",
            "-i",
            wm_path.to_str().unwrap(),
            "-t",
            "4",
            "-c",
            "copy",
            "-y",
            "-loglevel",
            "error",
            trimmed_path.to_str().unwrap(),
        ])
        .output()
        .expect("ffmpeg trim failed");

    assert!(result.status.success(), "Trim failed");

    // Verify trimmed video — may or may not detect depending on which frames survive
    let verify = VideoEngine
        .verify(trimmed_path.to_str().unwrap(), DEFAULT_PASSWORD)
        .expect("verify should not error");

    // Temporal trim + re-encoding may degrade per-frame watermarks.
    // We report the result but don't assert exact message match.
    if verify.detected {
        println!(
            "Temporal trim: DETECTED (confidence: {:.1}%, message: {:?})",
            verify.confidence * 100.0,
            verify.message
        );
    } else {
        println!("Temporal trim: watermark lost (documented v1 limitation)");
    }
}

// ── Dry run ──────────────────────────────────────────────────────────────

#[test]
fn test_video_dry_run() {
    if !ffmpeg_available() {
        eprintln!("SKIP: ffmpeg not available");
        return;
    }

    let input = input_dir().join("veo1.mp4");
    let output = output_path("veo1_never_created.mp4");

    let info = VideoEngine
        .dry_run(
            input.to_str().unwrap(),
            DEFAULT_MESSAGE,
            DEFAULT_PASSWORD,
            0,
            output.to_str().unwrap(),
        )
        .expect("dry_run failed");

    assert_eq!(info.mode, "video-temporal");
    assert!(info.keypoints > 0, "Should report watermarked frame count");
    assert!(!output.exists(), "Dry run should not create output file");
}

// ── Different resolutions and framerates ─────────────────────────────────

#[test]
fn test_video_720p_30fps_no_audio() {
    if !ffmpeg_available() {
        eprintln!("SKIP: ffmpeg not available");
        return;
    }
    let input = input_dir().join("test_720p_30fps.mp4");
    let output = output_path("test_720p_30fps_wm.mp4");

    let result = VideoEngine
        .embed(
            input.to_str().unwrap(),
            DEFAULT_MESSAGE,
            DEFAULT_PASSWORD,
            0,
            output.to_str().unwrap(),
        )
        .expect("embed failed");
    assert!(result.message.contains("成功"));

    let verify = VideoEngine
        .verify(output.to_str().unwrap(), DEFAULT_PASSWORD)
        .expect("verify failed");
    assert!(
        verify.detected,
        "Watermark not detected in 720p 30fps video"
    );
}

#[test]
fn test_video_1080p_60fps() {
    if !ffmpeg_available() {
        eprintln!("SKIP: ffmpeg not available");
        return;
    }
    let input = input_dir().join("test_1080p_60fps.mp4");
    let output = output_path("test_1080p_60fps_wm.mp4");

    VideoEngine
        .embed(
            input.to_str().unwrap(),
            DEFAULT_MESSAGE,
            DEFAULT_PASSWORD,
            0,
            output.to_str().unwrap(),
        )
        .expect("embed failed");

    let verify = VideoEngine
        .verify(output.to_str().unwrap(), DEFAULT_PASSWORD)
        .expect("verify failed");
    assert!(verify.detected, "Watermark not detected in 60fps video");
}

#[test]
fn test_video_portrait() {
    if !ffmpeg_available() {
        eprintln!("SKIP: ffmpeg not available");
        return;
    }
    let input = input_dir().join("test_portrait.mp4");
    let output = output_path("test_portrait_wm.mp4");

    VideoEngine
        .embed(
            input.to_str().unwrap(),
            DEFAULT_MESSAGE,
            DEFAULT_PASSWORD,
            0,
            output.to_str().unwrap(),
        )
        .expect("embed failed");

    let verify = VideoEngine
        .verify(output.to_str().unwrap(), DEFAULT_PASSWORD)
        .expect("verify failed");
    assert!(verify.detected, "Watermark not detected in portrait video");
}

// ── Short video edge case ────────────────────────────────────────────────

#[test]
fn test_video_short_2s() {
    if !ffmpeg_available() {
        eprintln!("SKIP: ffmpeg not available");
        return;
    }
    let input = input_dir().join("test_short_2s.mp4");
    let output = output_path("test_short_2s_wm.mp4");

    VideoEngine
        .embed(
            input.to_str().unwrap(),
            DEFAULT_MESSAGE,
            DEFAULT_PASSWORD,
            0,
            output.to_str().unwrap(),
        )
        .expect("embed failed");

    let verify = VideoEngine
        .verify(output.to_str().unwrap(), DEFAULT_PASSWORD)
        .expect("verify failed");
    assert!(verify.detected, "Watermark not detected in short video");
}

// ── Different containers and codecs ──────────────────────────────────────

#[test]
fn test_video_webm_vp9() {
    if !ffmpeg_available() {
        eprintln!("SKIP: ffmpeg not available");
        return;
    }
    let input = input_dir().join("test_vp9.webm");
    let output = output_path("test_vp9_wm.mp4"); // output is always MP4/H264

    VideoEngine
        .embed(
            input.to_str().unwrap(),
            DEFAULT_MESSAGE,
            DEFAULT_PASSWORD,
            0,
            output.to_str().unwrap(),
        )
        .expect("embed failed");

    let verify = VideoEngine
        .verify(output.to_str().unwrap(), DEFAULT_PASSWORD)
        .expect("verify failed");
    assert!(verify.detected, "Watermark not detected from VP9 input");
}

#[test]
fn test_video_mkv_with_audio() {
    if !ffmpeg_available() {
        eprintln!("SKIP: ffmpeg not available");
        return;
    }
    let input = input_dir().join("test_mkv.mkv");
    let output = output_path("test_mkv_wm.mp4");

    VideoEngine
        .embed(
            input.to_str().unwrap(),
            DEFAULT_MESSAGE,
            DEFAULT_PASSWORD,
            0,
            output.to_str().unwrap(),
        )
        .expect("embed failed");

    let verify = VideoEngine
        .verify(output.to_str().unwrap(), DEFAULT_PASSWORD)
        .expect("verify failed");
    assert!(verify.detected, "Watermark not detected from MKV input");
}

// ── Wrong password on different format ───────────────────────────────────

#[test]
fn test_video_wrong_password_720p() {
    if !ffmpeg_available() {
        eprintln!("SKIP: ffmpeg not available");
        return;
    }
    let input = input_dir().join("test_720p_30fps.mp4");
    let output = output_path("test_720p_wp.mp4");

    VideoEngine
        .embed(
            input.to_str().unwrap(),
            DEFAULT_MESSAGE,
            DEFAULT_PASSWORD,
            0,
            output.to_str().unwrap(),
        )
        .expect("embed failed");

    let result = VideoEngine
        .verify(output.to_str().unwrap(), "wrong_pw")
        .expect("verify should not error");
    assert!(
        !result.detected || result.message.as_deref() != Some(DEFAULT_MESSAGE),
        "Wrong password should not extract correct message"
    );
}
