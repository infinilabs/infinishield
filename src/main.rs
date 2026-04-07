use clap::{CommandFactory, Parser, Subcommand};

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
    infinishield verify -i watermarked.png\n  \
    infinishield verify -i watermarked.png -p secret\n\n\
    SUBCOMMANDS:\n\n  \
    embed   Embed a watermark into an image\n\n    \
      -i, --input <INPUT>          (required) Input image path (PNG, JPEG)\n    \
      -o, --output <OUTPUT>        (required) Output image path (PNG recommended)\n    \
      -m, --message <MESSAGE>      Message to embed [default: \"Copyright: InfiniLabs\"]\n    \
      -p, --password <PASSWORD>    Password for encryption [default: \"d1ng0\"]\n          \
      --intensity <INTENSITY>  Embedding intensity 1-10 [default: 5]\n\n  \
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
        #[arg(short = 'm', long, default_value = "Copyright: InfiniLabs")]
        message: String,

        /// Password for watermark encryption
        #[arg(short = 'p', long, default_value = "d1ng0")]
        password: String,

        /// Output image path (PNG recommended)
        #[arg(short = 'o', long)]
        output: String,

        /// Embedding intensity (1-10, higher = more robust but slightly more visible)
        #[arg(long, default_value_t = 5)]
        intensity: u8,
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

fn main() {
    let cli = Cli::parse();

    let command = match cli.command {
        Some(cmd) => cmd,
        None => {
            // No subcommand provided — print full help and exit 0
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
        } => {
            if !(1..=10).contains(&intensity) {
                eprintln!("[错误] 强度参数必须在 1-10 之间。");
                std::process::exit(1);
            }

            match infinishield::watermark::embed(&input, &message, &password, intensity, &output) {
                Ok(result) => {
                    println!("{}", result.message);
                }
                Err(e) => {
                    eprintln!("[错误] {}", e);
                    std::process::exit(1);
                }
            }
        }
        Commands::Verify { input, password } => {
            println!("[分析中] 正在执行频域扫描...");

            match infinishield::watermark::verify(&input, &password) {
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
