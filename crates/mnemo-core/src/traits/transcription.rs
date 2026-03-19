//! Transcription provider trait for audio-to-text conversion.
//!
//! The [`TranscriptionProvider`] trait defines the interface for transcribing
//! audio files to text using services like OpenAI Whisper, Deepgram, or
//! AssemblyAI.

use serde::{Deserialize, Serialize};

use crate::error::MnemoError;

pub type TranscriptionResult<T> = Result<T, MnemoError>;

/// Configuration for a transcription provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscriptionConfig {
    /// Provider name: "openai", "deepgram", "assemblyai", "local"
    pub provider: String,
    /// Model name (e.g., "whisper-1", "nova-2")
    pub model: String,
    /// API key (required for cloud providers).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    /// Base URL override (for custom endpoints).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    /// Enable speaker diarization (if supported by provider).
    #[serde(default)]
    pub diarization: bool,
    /// Language hint (ISO 639-1 code, e.g., "en", "es", "fr").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    /// Response format preference.
    #[serde(default)]
    pub response_format: TranscriptionFormat,
    /// Temperature for sampling (0.0 = deterministic).
    #[serde(default = "default_temperature")]
    pub temperature: f32,
}

fn default_temperature() -> f32 {
    0.0
}

impl Default for TranscriptionConfig {
    fn default() -> Self {
        Self {
            provider: "openai".to_string(),
            model: "whisper-1".to_string(),
            api_key: None,
            base_url: None,
            diarization: false,
            language: None,
            response_format: TranscriptionFormat::default(),
            temperature: default_temperature(),
        }
    }
}

/// Response format for transcription.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TranscriptionFormat {
    /// Plain text transcript.
    #[default]
    Text,
    /// JSON with word-level timestamps.
    Json,
    /// Verbose JSON with segments, timestamps, and metadata.
    VerboseJson,
    /// SRT subtitle format.
    Srt,
    /// VTT subtitle format.
    Vtt,
}

impl TranscriptionFormat {
    /// Get the format string for OpenAI API.
    pub fn openai_format(&self) -> &str {
        match self {
            Self::Text => "text",
            Self::Json => "json",
            Self::VerboseJson => "verbose_json",
            Self::Srt => "srt",
            Self::Vtt => "vtt",
        }
    }
}

/// Result of audio transcription.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Transcription {
    /// Full transcript text.
    pub text: String,

    /// Language detected or specified.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,

    /// Duration of the audio in seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_secs: Option<f32>,

    /// Transcript segments with timestamps.
    #[serde(default)]
    pub segments: Vec<TranscriptSegment>,

    /// Words with timestamps (if available).
    #[serde(default)]
    pub words: Vec<TranscriptWord>,

    /// Speakers identified (for diarization).
    #[serde(default)]
    pub speakers: Vec<Speaker>,

    /// Usage/billing information.
    pub usage: TranscriptionUsage,
}

/// A segment of the transcript with timing information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscriptSegment {
    /// Segment ID.
    pub id: u32,
    /// Start time in seconds.
    pub start: f32,
    /// End time in seconds.
    pub end: f32,
    /// Transcript text for this segment.
    pub text: String,
    /// Speaker ID (if diarization enabled).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speaker: Option<String>,
    /// Confidence score (0.0 - 1.0).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f32>,
}

/// A word with timing information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscriptWord {
    /// The word.
    pub word: String,
    /// Start time in seconds.
    pub start: f32,
    /// End time in seconds.
    pub end: f32,
    /// Confidence score (0.0 - 1.0).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f32>,
}

/// Speaker information for diarization.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Speaker {
    /// Speaker ID (e.g., "SPEAKER_00", "SPEAKER_01").
    pub id: String,
    /// Speaker label (if identified).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

/// Token/billing usage for transcription.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TranscriptionUsage {
    /// Audio duration in seconds.
    pub audio_seconds: f32,
    /// Billable seconds (may differ from duration due to rounding).
    #[serde(default)]
    pub billable_seconds: f32,
}

/// Supported audio formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AudioFormat {
    /// MP3 audio.
    Mp3,
    /// MP4 audio (M4A).
    Mp4,
    /// MPEG audio.
    Mpeg,
    /// MPGA audio.
    Mpga,
    /// WAV audio.
    Wav,
    /// WebM audio.
    Webm,
    /// FLAC audio.
    Flac,
    /// OGG audio.
    Ogg,
}

impl AudioFormat {
    /// Get the MIME type for this format.
    pub fn media_type(&self) -> &str {
        match self {
            Self::Mp3 => "audio/mpeg",
            Self::Mp4 => "audio/mp4",
            Self::Mpeg => "audio/mpeg",
            Self::Mpga => "audio/mpeg",
            Self::Wav => "audio/wav",
            Self::Webm => "audio/webm",
            Self::Flac => "audio/flac",
            Self::Ogg => "audio/ogg",
        }
    }

    /// Parse audio format from MIME type.
    pub fn from_mime(mime: &str) -> Option<Self> {
        let mime_lower = mime.to_lowercase();
        match mime_lower.as_str() {
            "audio/mpeg" | "audio/mp3" => Some(Self::Mp3),
            "audio/mp4" | "audio/m4a" | "audio/x-m4a" => Some(Self::Mp4),
            "audio/wav" | "audio/x-wav" | "audio/wave" => Some(Self::Wav),
            "audio/webm" => Some(Self::Webm),
            "audio/flac" | "audio/x-flac" => Some(Self::Flac),
            "audio/ogg" | "audio/vorbis" => Some(Self::Ogg),
            _ => None,
        }
    }

    /// Get the file extension for this format.
    pub fn extension(&self) -> &str {
        match self {
            Self::Mp3 => "mp3",
            Self::Mp4 => "m4a",
            Self::Mpeg => "mpeg",
            Self::Mpga => "mpga",
            Self::Wav => "wav",
            Self::Webm => "webm",
            Self::Flac => "flac",
            Self::Ogg => "ogg",
        }
    }
}

/// Trait for transcription providers.
///
/// Implement this trait to add support for different transcription services
/// (OpenAI Whisper, Deepgram, AssemblyAI, local models, etc.).
#[allow(async_fn_in_trait)]
pub trait TranscriptionProvider: Send + Sync {
    /// Transcribe audio data to text.
    ///
    /// # Arguments
    /// * `audio_data` - Raw audio bytes.
    /// * `format` - The audio format (MP3, WAV, etc.).
    /// * `filename` - Optional filename hint for the API.
    ///
    /// # Returns
    /// A `Transcription` containing the transcript text and metadata.
    async fn transcribe(
        &self,
        audio_data: &[u8],
        format: AudioFormat,
        filename: Option<&str>,
    ) -> TranscriptionResult<Transcription>;

    /// Get the provider name for logging/debugging.
    fn provider_name(&self) -> &str;

    /// Get the model name being used.
    fn model_name(&self) -> &str;

    /// Check if the provider supports a given audio format.
    fn supports_format(&self, format: AudioFormat) -> bool {
        // OpenAI Whisper supports these formats
        matches!(
            format,
            AudioFormat::Mp3
                | AudioFormat::Mp4
                | AudioFormat::Mpeg
                | AudioFormat::Mpga
                | AudioFormat::Wav
                | AudioFormat::Webm
                | AudioFormat::Flac
                | AudioFormat::Ogg
        )
    }

    /// Get the maximum supported audio file size in bytes.
    fn max_audio_size(&self) -> u64 {
        25 * 1024 * 1024 // 25 MB default (OpenAI Whisper limit)
    }

    /// Check if diarization (speaker identification) is supported.
    fn supports_diarization(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_audio_format_from_mime() {
        assert_eq!(AudioFormat::from_mime("audio/mpeg"), Some(AudioFormat::Mp3));
        assert_eq!(AudioFormat::from_mime("audio/mp3"), Some(AudioFormat::Mp3));
        assert_eq!(AudioFormat::from_mime("audio/mp4"), Some(AudioFormat::Mp4));
        assert_eq!(AudioFormat::from_mime("audio/wav"), Some(AudioFormat::Wav));
        assert_eq!(
            AudioFormat::from_mime("audio/webm"),
            Some(AudioFormat::Webm)
        );
        assert_eq!(
            AudioFormat::from_mime("audio/flac"),
            Some(AudioFormat::Flac)
        );
        assert_eq!(AudioFormat::from_mime("audio/ogg"), Some(AudioFormat::Ogg));
        assert_eq!(AudioFormat::from_mime("video/mp4"), None);
        assert_eq!(AudioFormat::from_mime("text/plain"), None);
    }

    #[test]
    fn test_audio_format_media_type() {
        assert_eq!(AudioFormat::Mp3.media_type(), "audio/mpeg");
        assert_eq!(AudioFormat::Wav.media_type(), "audio/wav");
        assert_eq!(AudioFormat::Flac.media_type(), "audio/flac");
    }

    #[test]
    fn test_transcription_format_openai() {
        assert_eq!(TranscriptionFormat::Text.openai_format(), "text");
        assert_eq!(TranscriptionFormat::Json.openai_format(), "json");
        assert_eq!(
            TranscriptionFormat::VerboseJson.openai_format(),
            "verbose_json"
        );
        assert_eq!(TranscriptionFormat::Srt.openai_format(), "srt");
        assert_eq!(TranscriptionFormat::Vtt.openai_format(), "vtt");
    }

    #[test]
    fn test_transcription_config_default() {
        let config = TranscriptionConfig::default();
        assert_eq!(config.provider, "openai");
        assert_eq!(config.model, "whisper-1");
        assert!(!config.diarization);
        assert_eq!(config.temperature, 0.0);
    }
}
