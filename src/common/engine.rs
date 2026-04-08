/// Metadata about an embedding operation (returned by both embed and dry_run).
#[derive(Debug, Clone)]
pub struct EmbedInfo {
    /// Human-readable status message.
    pub status: String,
    /// Embedding mode used: "feature-point" or "global-dwt".
    pub mode: String,
    /// Message that was (or would be) embedded.
    pub message: String,
    /// Message length in bytes.
    pub message_bytes: usize,
    /// Resolved intensity (1-10).
    pub intensity: u8,
    /// Image width in pixels.
    pub width: u32,
    /// Image height in pixels.
    pub height: u32,
    /// Number of feature keypoints detected (0 for global DWT mode).
    pub keypoints: usize,
    /// Maximum message capacity in bytes for the chosen mode.
    pub max_capacity: usize,
    /// Output file path.
    pub output_path: String,
}

impl EmbedInfo {
    /// Format a summary string for console output.
    pub fn summary(&self) -> String {
        let robustness = match self.intensity {
            1..=3 => "低",
            4..=7 => "中",
            _ => "高",
        };
        format!(
            "[成功] 水印已嵌入。\n\
             [信息] 模式: {} | 消息: \"{}\" ({} 字节) | 强度: {} | 抗压缩率: {}\n\
             [信息] 图像: {}x{} | 特征点: {} | 容量上限: {} 字节\n\
             [信息] 输出: {}",
            self.mode,
            self.message,
            self.message_bytes,
            self.intensity,
            robustness,
            self.width,
            self.height,
            self.keypoints,
            self.max_capacity,
            self.output_path,
        )
    }

    /// Format a dry-run summary (no file written).
    pub fn dry_run_summary(&self) -> String {
        let robustness = match self.intensity {
            1..=3 => "低",
            4..=7 => "中",
            _ => "高",
        };
        format!(
            "[模拟] 水印嵌入预览 (未生成文件):\n\
             [信息] 模式: {} | 消息: \"{}\" ({} 字节) | 强度: {} | 抗压缩率: {}\n\
             [信息] 图像: {}x{} | 特征点: {} | 容量上限: {} 字节\n\
             [信息] 输出: {}",
            self.mode,
            self.message,
            self.message_bytes,
            self.intensity,
            robustness,
            self.width,
            self.height,
            self.keypoints,
            self.max_capacity,
            self.output_path,
        )
    }
}

/// Result of a successful watermark embedding (kept for backward compat).
#[derive(Debug)]
pub struct EmbedResult {
    /// Human-readable status message.
    pub message: String,
    /// Detailed embedding info.
    pub info: EmbedInfo,
}

/// Result of a watermark verification/extraction attempt.
#[derive(Debug)]
pub struct ExtractResult {
    /// Whether a valid watermark was detected.
    pub detected: bool,
    /// Detection confidence (0.0 to 1.0).
    pub confidence: f64,
    /// Extracted message, if decoding succeeded.
    pub message: Option<String>,
}

/// Uniform interface for all watermark engines (raster, vector, video).
///
/// Each engine implements format-specific feature detection and embedding
/// while sharing the common layer (ECC, scrambling, password hashing).
pub trait WatermarkEngine {
    /// Embed a watermark message into a file.
    fn embed(
        &self,
        input_path: &str,
        message: &str,
        password: &str,
        intensity: u8,
        output_path: &str,
    ) -> Result<EmbedResult, String>;

    /// Dry run: compute embedding info without writing any file.
    fn dry_run(
        &self,
        input_path: &str,
        message: &str,
        password: &str,
        intensity: u8,
        output_path: &str,
    ) -> Result<EmbedInfo, String>;

    /// Verify and extract a watermark from a file.
    fn verify(&self, input_path: &str, password: &str) -> Result<ExtractResult, String>;
}
