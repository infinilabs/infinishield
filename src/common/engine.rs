/// Result of a successful watermark embedding.
#[derive(Debug)]
pub struct EmbedResult {
    /// Human-readable status message.
    pub message: String,
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

    /// Verify and extract a watermark from a file.
    fn verify(&self, input_path: &str, password: &str) -> Result<ExtractResult, String>;
}
