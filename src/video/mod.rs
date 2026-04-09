//! Video engine — single-pass streaming watermarking via ffmpeg-next.
#![allow(clippy::manual_is_multiple_of)]
//!
//! Architecture: O(1) memory, zero disk I/O for frame data.
//!
//! Pipeline (single loop):
//!   1. Read packet from input
//!   2. If video: decode → RGB → conditionally watermark via embed_buffer → YUV → encode → write
//!   3. If audio: copy packet directly to output (same loop, no second pass)
//!   4. Repeat until EOF
//!
//! Only 1-2 frames are in memory at any time.
//! Watermarks 1 keyframe per second using RasterEngine's in-memory buffer API.
//! Re-encodes with H.264 (libx264, CRF 18).
//!
//! Requires the `video` cargo feature + libx264-dev.

#![allow(clippy::too_many_arguments)]

extern crate ffmpeg_next as ffmpeg;

use crate::common::engine::{EmbedInfo, EmbedResult, ExtractResult, WatermarkEngine};
use crate::raster::RasterEngine;

use ffmpeg::format::Pixel;
use ffmpeg::software::scaling::{context::Context as ScalerCtx, flag::Flags as ScalerFlags};
use ffmpeg::util::frame::video::Video;
use ffmpeg::{codec, encoder, format, media, picture, Dictionary, Packet, Rational};

use std::collections::HashMap;

const VIDEO_MAX_MESSAGE: usize = 7;

pub struct VideoEngine;

impl WatermarkEngine for VideoEngine {
    fn embed(
        &self,
        input_path: &str,
        message: &str,
        password: &str,
        intensity: u8,
        output_path: &str,
    ) -> Result<EmbedResult, String> {
        ffmpeg::init().map_err(|e| format!("FFmpeg init: {}", e))?;
        if message.len() > VIDEO_MAX_MESSAGE {
            return Err(format!(
                "Message too long: max {} bytes, got {}",
                VIDEO_MAX_MESSAGE,
                message.len()
            ));
        }

        let wm_count = streaming_transcode(
            input_path,
            output_path,
            Some((message, password, intensity)),
        )?;

        let info = EmbedInfo {
            status: "ok".to_string(),
            mode: "video-temporal".to_string(),
            message: message.to_string(),
            message_bytes: message.len(),
            intensity,
            width: 0,
            height: 0,
            keypoints: wm_count,
            max_capacity: VIDEO_MAX_MESSAGE,
            output_path: output_path.to_string(),
        };
        Ok(EmbedResult {
            message: info.summary(),
            info,
        })
    }

    fn dry_run(
        &self,
        input_path: &str,
        message: &str,
        _password: &str,
        intensity: u8,
        output_path: &str,
    ) -> Result<EmbedInfo, String> {
        ffmpeg::init().map_err(|e| format!("FFmpeg init: {}", e))?;
        if message.len() > VIDEO_MAX_MESSAGE {
            return Err(format!(
                "Message too long: {} bytes, max: {}",
                message.len(),
                VIDEO_MAX_MESSAGE
            ));
        }
        let ictx = format::input(input_path).map_err(|e| format!("Open: {}", e))?;
        let _vs = ictx.streams().best(media::Type::Video).ok_or("No video")?;
        let dur = ictx.duration() as f64 / ffmpeg::ffi::AV_TIME_BASE as f64;

        Ok(EmbedInfo {
            status: "ok".to_string(),
            mode: "video-temporal".to_string(),
            message: message.to_string(),
            message_bytes: message.len(),
            intensity,
            width: 0,
            height: 0,
            keypoints: (dur.ceil() as usize).max(1),
            max_capacity: VIDEO_MAX_MESSAGE,
            output_path: output_path.to_string(),
        })
    }

    fn verify(&self, input_path: &str, password: &str) -> Result<ExtractResult, String> {
        ffmpeg::init().map_err(|e| format!("FFmpeg init: {}", e))?;

        let mut ictx = format::input(input_path).map_err(|e| format!("Open: {}", e))?;
        let vs = ictx.streams().best(media::Type::Video).ok_or("No video")?;
        let vi = vs.index();
        let fps = f64::from(vs.avg_frame_rate()).max(1.0);
        let interval = (fps.round() as usize).max(1);

        let ctx = ffmpeg::codec::context::Context::from_parameters(vs.parameters())
            .map_err(|e| e.to_string())?;
        let mut dec = ctx.decoder().video().map_err(|e| e.to_string())?;
        let w = dec.width();
        let h = dec.height();

        let mut scaler = ScalerCtx::get(
            dec.format(),
            w,
            h,
            Pixel::RGB24,
            w,
            h,
            ScalerFlags::BILINEAR,
        )
        .map_err(|e| e.to_string())?;

        let raster = RasterEngine;
        let mut detected: Vec<(String, f64)> = Vec::new();
        let mut frame_idx = 0usize;

        let mut process = |d: &mut ffmpeg::decoder::Video| -> Result<(), String> {
            let mut decoded = Video::empty();
            while d.receive_frame(&mut decoded).is_ok() {
                if frame_idx % interval == 0 {
                    let mut rgb = Video::empty();
                    scaler.run(&decoded, &mut rgb).map_err(|e| e.to_string())?;
                    let buf = frame_to_rgb_vec(&rgb, w, h);
                    if let Ok(r) = raster.verify_buffer(&buf, w, h, password) {
                        if r.detected {
                            if let Some(msg) = r.message {
                                detected.push((msg, r.confidence));
                            }
                        }
                    }
                }
                frame_idx += 1;
            }
            Ok(())
        };

        for (stream, packet) in ictx.packets() {
            if stream.index() == vi {
                dec.send_packet(&packet).map_err(|e| e.to_string())?;
                process(&mut dec)?;
            }
        }
        dec.send_eof().map_err(|e| e.to_string())?;
        process(&mut dec)?;

        if detected.is_empty() {
            return Ok(ExtractResult {
                detected: false,
                confidence: 0.0,
                message: None,
            });
        }

        let mut counts: HashMap<String, (usize, f64)> = HashMap::new();
        for (msg, conf) in &detected {
            let e = counts.entry(msg.clone()).or_insert((0, 0.0));
            e.0 += 1;
            e.1 += conf;
        }
        let (best, (cnt, total)) = counts.into_iter().max_by_key(|(_, (c, _))| *c).unwrap();

        Ok(ExtractResult {
            detected: true,
            confidence: total / cnt as f64,
            message: Some(best),
        })
    }
}

// ── Single-Pass Streaming Transcode ──────────────────────────────────────

fn streaming_transcode(
    input_path: &str,
    output_path: &str,
    watermark: Option<(&str, &str, u8)>,
) -> Result<usize, String> {
    let mut ictx = format::input(input_path).map_err(|e| format!("Input: {}", e))?;
    let mut octx = format::output(output_path).map_err(|e| format!("Output: {}", e))?;

    // Video stream setup
    let video_ist = ictx.streams().best(media::Type::Video).ok_or("No video")?;
    let video_idx = video_ist.index();
    let fps = f64::from(video_ist.avg_frame_rate()).max(1.0);
    let interval = (fps.round() as usize).max(1);
    let video_tb = video_ist.time_base();

    let dec_ctx = ffmpeg::codec::context::Context::from_parameters(video_ist.parameters())
        .map_err(|e| e.to_string())?;
    let mut decoder = dec_ctx.decoder().video().map_err(|e| e.to_string())?;
    let w = decoder.width();
    let h = decoder.height();

    // H.264 encoder
    let enc_codec = encoder::find(codec::Id::H264);
    let gh = octx.format().flags().contains(format::Flags::GLOBAL_HEADER);

    let video_ost_idx;
    let mut video_enc;
    {
        let mut ost = octx.add_stream(enc_codec).map_err(|e| e.to_string())?;
        let mut e = codec::context::Context::new_with_codec(
            enc_codec.ok_or("H264 not found. Install libx264-dev.")?,
        )
        .encoder()
        .video()
        .map_err(|e| e.to_string())?;

        ost.set_parameters(&e);
        e.set_width(w);
        e.set_height(h);
        e.set_format(Pixel::YUV420P);
        e.set_time_base(video_tb);
        e.set_frame_rate(Some(Rational(fps.round() as i32, 1)));
        if gh {
            e.set_flags(codec::Flags::GLOBAL_HEADER);
        }
        let mut opts = Dictionary::new();
        opts.set("preset", "medium");
        opts.set("crf", "18");
        video_enc = e.open_with(opts).map_err(|e| e.to_string())?;
        ost.set_parameters(&video_enc);
        video_ost_idx = ost.index();
    }

    // Audio stream copy
    let mut audio_map: Option<(usize, usize)> = None;
    for ist in ictx.streams() {
        if ist.parameters().medium() == media::Type::Audio {
            let mut ost = octx
                .add_stream(encoder::find(codec::Id::None))
                .map_err(|e| e.to_string())?;
            ost.set_parameters(ist.parameters());
            unsafe {
                (*ost.parameters().as_mut_ptr()).codec_tag = 0;
            }
            audio_map = Some((ist.index(), ost.index()));
            break;
        }
    }

    octx.write_header().map_err(|e| e.to_string())?;

    let video_ost_tb = octx.stream(video_ost_idx).unwrap().time_base();
    let audio_ost_tb = audio_map.map(|(_, ao)| octx.stream(ao).unwrap().time_base());

    // Scalers: YUV↔RGB
    let mut to_rgb = ScalerCtx::get(
        decoder.format(),
        w,
        h,
        Pixel::RGB24,
        w,
        h,
        ScalerFlags::BILINEAR,
    )
    .map_err(|e| e.to_string())?;

    let mut to_yuv = ScalerCtx::get(
        Pixel::RGB24,
        w,
        h,
        Pixel::YUV420P,
        w,
        h,
        ScalerFlags::BILINEAR,
    )
    .map_err(|e| e.to_string())?;

    let raster = RasterEngine;
    let mut frame_idx = 0usize;
    let mut wm_count = 0usize;

    // Collect audio input time bases before the loop
    let audio_ist_tbs: HashMap<usize, Rational> =
        ictx.streams().map(|s| (s.index(), s.time_base())).collect();

    // Closure: decode → watermark → encode → write
    let mut process_decoded = |dec: &mut ffmpeg::decoder::Video,
                               enc: &mut encoder::Video,
                               out: &mut format::context::Output|
     -> Result<(), String> {
        let mut decoded = Video::empty();
        while dec.receive_frame(&mut decoded).is_ok() {
            let mut rgb = Video::empty();
            to_rgb.run(&decoded, &mut rgb).map_err(|e| e.to_string())?;

            // Conditionally watermark this frame
            if let Some((msg, pwd, intensity)) = watermark {
                if frame_idx % interval == 0 {
                    let mut buf = frame_to_rgb_vec(&rgb, w, h);
                    if raster
                        .embed_buffer(&mut buf, w, h, msg, pwd, intensity)
                        .is_ok()
                    {
                        rgb_vec_to_frame(&buf, &mut rgb, w, h);
                        wm_count += 1;
                    }
                }
            }

            let mut yuv = Video::empty();
            to_yuv.run(&rgb, &mut yuv).map_err(|e| e.to_string())?;
            yuv.set_pts(Some(frame_idx as i64));
            yuv.set_kind(picture::Type::None);

            enc.send_frame(&yuv).map_err(|e| e.to_string())?;
            let mut pkt = Packet::empty();
            while enc.receive_packet(&mut pkt).is_ok() {
                pkt.set_stream(video_ost_idx);
                pkt.rescale_ts(video_tb, video_ost_tb);
                pkt.write_interleaved(out).map_err(|e| e.to_string())?;
            }
            frame_idx += 1;
        }
        Ok(())
    };

    // ── THE SINGLE-PASS LOOP ──
    for (stream, mut packet) in ictx.packets() {
        let idx = stream.index();

        if idx == video_idx {
            decoder.send_packet(&packet).map_err(|e| e.to_string())?;
            process_decoded(&mut decoder, &mut video_enc, &mut octx)?;
        } else if let Some((ai, ao)) = audio_map {
            if idx == ai {
                let ist_tb = audio_ist_tbs[&ai];
                let ost_tb = audio_ost_tb.unwrap();
                packet.rescale_ts(ist_tb, ost_tb);
                packet.set_position(-1);
                packet.set_stream(ao);
                let _ = packet.write_interleaved(&mut octx);
            }
        }
    }

    // Flush
    decoder.send_eof().map_err(|e| e.to_string())?;
    process_decoded(&mut decoder, &mut video_enc, &mut octx)?;

    video_enc.send_eof().map_err(|e| e.to_string())?;
    let mut pkt = Packet::empty();
    while video_enc.receive_packet(&mut pkt).is_ok() {
        pkt.set_stream(video_ost_idx);
        pkt.rescale_ts(video_tb, video_ost_tb);
        pkt.write_interleaved(&mut octx)
            .map_err(|e| e.to_string())?;
    }

    octx.write_trailer().map_err(|e| e.to_string())?;
    Ok(wm_count)
}

fn frame_to_rgb_vec(frame: &Video, w: u32, h: u32) -> Vec<u8> {
    let stride = frame.stride(0);
    let mut buf = Vec::with_capacity((w * h * 3) as usize);
    for y in 0..h as usize {
        buf.extend_from_slice(&frame.data(0)[(y * stride)..][..w as usize * 3]);
    }
    buf
}

fn rgb_vec_to_frame(buf: &[u8], frame: &mut Video, w: u32, h: u32) {
    let stride = frame.stride(0);
    for y in 0..h as usize {
        let src = &buf[(y * w as usize * 3)..][..w as usize * 3];
        frame.data_mut(0)[(y * stride)..][..w as usize * 3].copy_from_slice(src);
    }
}
