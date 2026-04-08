use std::path::Path;

use clap::{CommandFactory, Parser, Subcommand};
use infinishield::common::WatermarkEngine;
use infinishield::raster::RasterEngine;

#[derive(Parser)]
#[command(name = "infinishield")]
#[command(version)]
#[command(about = "Invisible robust watermarking system for multimedia files")]
#[command(
    long_about = "infinishield embeds invisible, robust watermarks into images using \
    frequency-domain techniques (DWT + spread spectrum).\n\n\
    Watermarks survive compression and noise, and can only be extracted \
    with the correct password. Supports PNG and JPEG input; output is always PNG.\n\n\
    EXAMPLES:\n  \
    infinishield embed -i photo.jpg -o watermarked.png\n  \
    infinishield embed -i photo.jpg -m \"My Copyright\" -p secret -o out.png --intensity 8\n  \
    infinishield embed -i photo.jpg -o out.png --dry-run\n  \
    infinishield verify -i watermarked.png\n  \
    infinishield verify -i watermarked.png -p secret\n\n\
    SUBCOMMANDS:\n\n  \
    embed   Embed a watermark into an image\n\n    \
      -i, --input <INPUT>          (required) Input image path (PNG, JPEG)\n    \
      -o, --output <OUTPUT>        (required) Output image path (PNG recommended)\n    \
      -m, --message <MESSAGE>      Message to embed [default: \"Infini\"]\n    \
      -p, --password <PASSWORD>    Password for encryption [default: \"d1ng0\"]\n          \
      --intensity <INTENSITY>  Embedding intensity 1-10 [default: auto]\n          \
      --dry-run                Preview embedding without writing output\n\n  \
    verify  Verify and extract a watermark from an image\n\n    \
      -i, --input <INPUT>          (required) Input image path\n    \
      -p, --password <PASSWORD>    Password used during embedding [default: \"d1ng0\"]"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Embed a watermark into an image
    Embed {
        /// Input image path (supports PNG, JPEG)
        #[arg(short = 'i', long)]
        input: String,

        /// Message to embed as watermark
        #[arg(short = 'm', long, default_value = "Infini")]
        message: String,

        /// Password for watermark encryption
        #[arg(short = 'p', long, default_value = "d1ng0")]
        password: String,

        /// Output image path (PNG recommended)
        #[arg(short = 'o', long)]
        output: String,

        /// Embedding intensity (1-10). Auto-selected from image size if omitted
        #[arg(long)]
        intensity: Option<u8>,

        /// Preview embedding info without writing the output file
        #[arg(long)]
        dry_run: bool,
    },

    /// Verify and extract a watermark from an image
    Verify {
        /// Input image path to verify
        #[arg(short = 'i', long)]
        input: String,

        /// Password used during embedding
        #[arg(short = 'p', long, default_value = "d1ng0")]
        password: String,
    },
}

/// Detect file format and return the appropriate engine.
fn engine_for_file(path: &str) -> Result<Box<dyn WatermarkEngine>, String> {
    let ext = Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .unwrap_or_default();

    match ext.as_str() {
        "jpg" | "jpeg" | "png" | "webp" | "bmp" | "tiff" | "tif" | "gif" => {
            Ok(Box::new(RasterEngine))
        }
        _ => Err(format!(
            "Unsupported file format: .{}. Supported: jpg, jpeg, png, webp, bmp, tiff, gif",
            ext
        )),
    }
}

/// Warn the user if the output format is lossy, which may degrade the watermark.
fn warn_if_lossy_output(output_path: &str) {
    let ext = Path::new(output_path)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .unwrap_or_default();

    match ext.as_str() {
        "jpg" | "jpeg" => {
            eprintln!("[警告] 输出格式为 JPEG (有损压缩)，水印可能被压缩降质。建议使用 PNG 格式。");
        }
        "webp" => {
            eprintln!(
                "[警告] 输出格式为 WebP，可能使用有损压缩，水印可能被降质。建议使用 PNG 格式。"
            );
        }
        "gif" => {
            eprintln!("[警告] 输出格式为 GIF (仅 256 色)，水印会严重降质。建议使用 PNG 格式。");
        }
        _ => {} // png, bmp, tiff — lossless, no warning
    }
}

fn main() {
    let cli = Cli::parse();

    let command = match cli.command {
        Some(cmd) => cmd,
        None => {
            Cli::command().print_long_help().unwrap();
            println!();
            std::process::exit(0);
        }
    };

    match command {
        Commands::Embed {
            input,
            message,
            password,
            output,
            intensity,
            dry_run,
        } => {
            let intensity = match intensity {
                Some(v) => {
                    if !(1..=10).contains(&v) {
                        eprintln!("[错误] 强度参数必须在 1-10 之间。");
                        std::process::exit(1);
                    }
                    v
                }
                None => 0, // auto
            };

            // Warn if output format is lossy
            warn_if_lossy_output(&output);

            let engine = match engine_for_file(&input) {
                Ok(e) => e,
                Err(e) => {
                    eprintln!("[错误] {}", e);
                    std::process::exit(1);
                }
            };

            if dry_run {
                match engine.dry_run(&input, &message, &password, intensity, &output) {
                    Ok(info) => {
                        println!("{}", info.dry_run_summary());
                    }
                    Err(e) => {
                        eprintln!("[错误] {}", e);
                        std::process::exit(1);
                    }
                }
            } else {
                match engine.embed(&input, &message, &password, intensity, &output) {
                    Ok(result) => {
                        println!("{}", result.message);
                    }
                    Err(e) => {
                        eprintln!("[错误] {}", e);
                        std::process::exit(1);
                    }
                }
            }
        }
        Commands::Verify { input, password } => {
            println!("[分析中] 正在执行频域扫描...");

            let engine = match engine_for_file(&input) {
                Ok(e) => e,
                Err(e) => {
                    eprintln!("[错误] {}", e);
                    std::process::exit(1);
                }
            };

            match engine.verify(&input, &password) {
                Ok(result) => {
                    if result.detected {
                        println!(
                            "[验证结果] 匹配成功！(置信度: {:.1}%)",
                            result.confidence * 100.0
                        );
                        if let Some(msg) = result.message {
                            println!("[提取内容] \"{}\"", msg);
                        }
                    } else {
                        println!("[验证结果] 失败。未检测到有效水印，或密码错误。");
                    }
                }
                Err(e) => {
                    eprintln!("[错误] {}", e);
                    std::process::exit(1);
                }
            }
        }
    }
}
