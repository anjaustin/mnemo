//! OpenAI Whisper transcription provider implementation.
//!
//! Uses the OpenAI Audio API for transcription with the Whisper model.
//! Supports multiple audio formats and optional language hints.

use reqwest::{multipart, Client};
use serde::Deserialize;
use tracing::{debug, instrument};

use mnemo_core::error::MnemoError;
use mnemo_core::traits::transcription::{
    AudioFormat, TranscriptSegment, TranscriptWord, Transcription, TranscriptionConfig,
    TranscriptionProvider, TranscriptionResult, TranscriptionUsage,
};

/// OpenAI Whisper transcription provider.
///
/// Supports the OpenAI Audio API with Whisper models:
/// - whisper-1 (default, best quality)
///
/// Also works with OpenAI-compatible APIs that support the
/// /v1/audio/transcriptions endpoint.
pub struct OpenAITranscriptionProvider {
    client: Client,
    config: TranscriptionConfig,
    base_url: String,
}

// ─── Response Types ────────────────────────────────────────────────

#[derive(Deserialize)]
struct VerboseTranscriptionResponse {
    text: String,
    #[serde(default)]
    language: Option<String>,
    #[serde(default)]
    duration: Option<f32>,
    #[serde(default)]
    segments: Vec<WhisperSegment>,
    #[serde(default)]
    words: Vec<WhisperWord>,
}

#[derive(Deserialize)]
struct WhisperSegment {
    id: u32,
    start: f32,
    end: f32,
    text: String,
    #[serde(default)]
    avg_logprob: Option<f32>,
}

#[derive(Deserialize)]
struct WhisperWord {
    word: String,
    start: f32,
    end: f32,
}

// ─── Implementation ────────────────────────────────────────────────

impl OpenAITranscriptionProvider {
    /// Create a new OpenAI transcription provider.
    pub fn new(config: TranscriptionConfig) -> Self {
        let base_url = config
            .base_url
            .clone()
            .unwrap_or_else(|| "https://api.openai.com".to_string());
        let client = Client::new();
        Self {
            client,
            config,
            base_url,
        }
    }

    /// Create from environment variables.
    ///
    /// Reads:
    /// - `OPENAI_API_KEY` or `MNEMO_TRANSCRIPTION_API_KEY`
    /// - `MNEMO_TRANSCRIPTION_MODEL` (default: "whisper-1")
    pub fn from_env() -> Result<Self, MnemoError> {
        let api_key = std::env::var("MNEMO_TRANSCRIPTION_API_KEY")
            .or_else(|_| std::env::var("OPENAI_API_KEY"))
            .map_err(|_| {
                MnemoError::Config(
                    "MNEMO_TRANSCRIPTION_API_KEY or OPENAI_API_KEY is required".to_string(),
                )
            })?;

        let model = std::env::var("MNEMO_TRANSCRIPTION_MODEL")
            .unwrap_or_else(|_| "whisper-1".to_string());

        let language = std::env::var("MNEMO_TRANSCRIPTION_LANGUAGE").ok();

        let config = TranscriptionConfig {
            provider: "openai".to_string(),
            model,
            api_key: Some(api_key),
            language,
            ..Default::default()
        };

        Ok(Self::new(config))
    }
}

impl TranscriptionProvider for OpenAITranscriptionProvider {
    #[instrument(skip(self, audio_data), fields(format = ?format, size = audio_data.len()))]
    async fn transcribe(
        &self,
        audio_data: &[u8],
        format: AudioFormat,
        filename: Option<&str>,
    ) -> TranscriptionResult<Transcription> {
        let api_key = self.config.api_key.as_deref().ok_or_else(|| {
            MnemoError::LlmProvider {
                provider: "openai_transcription".into(),
                message: "API key is required".into(),
            }
        })?;

        // Determine filename for the multipart upload
        let file_name = filename
            .map(|s| s.to_string())
            .unwrap_or_else(|| format!("audio.{}", format.extension()));

        // Build multipart form
        let file_part = multipart::Part::bytes(audio_data.to_vec())
            .file_name(file_name)
            .mime_str(format.media_type())
            .map_err(|e| MnemoError::LlmProvider {
                provider: "openai_transcription".into(),
                message: format!("Invalid MIME type: {}", e),
            })?;

        let mut form = multipart::Form::new()
            .part("file", file_part)
            .text("model", self.config.model.clone())
            .text("response_format", "verbose_json"); // Always request verbose for segments

        // Add optional language hint
        if let Some(lang) = &self.config.language {
            form = form.text("language", lang.clone());
        }

        // Add temperature
        if self.config.temperature > 0.0 {
            form = form.text("temperature", self.config.temperature.to_string());
        }

        debug!(
            model = %self.config.model,
            audio_size = audio_data.len(),
            "Sending transcription request to OpenAI"
        );

        let response = self
            .client
            .post(format!("{}/v1/audio/transcriptions", self.base_url))
            .header("Authorization", format!("Bearer {}", api_key))
            .multipart(form)
            .send()
            .await
            .map_err(|e| MnemoError::LlmProvider {
                provider: "openai_transcription".into(),
                message: format!("Request failed: {}", e),
            })?;

        let status = response.status();
        if !status.is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(MnemoError::LlmProvider {
                provider: "openai_transcription".into(),
                message: format!("API error {}: {}", status, error_text),
            });
        }

        // Parse verbose JSON response
        let api_response: VerboseTranscriptionResponse = response.json().await.map_err(|e| {
            MnemoError::LlmProvider {
                provider: "openai_transcription".into(),
                message: format!("Failed to parse response: {}", e),
            }
        })?;

        // Convert segments
        let segments: Vec<TranscriptSegment> = api_response
            .segments
            .into_iter()
            .map(|s| {
                // Convert log probability to confidence (0-1 range)
                let confidence = s.avg_logprob.map(|lp| (lp.exp()).min(1.0).max(0.0));
                TranscriptSegment {
                    id: s.id,
                    start: s.start,
                    end: s.end,
                    text: s.text.trim().to_string(),
                    speaker: None, // Whisper doesn't provide speaker diarization
                    confidence,
                }
            })
            .collect();

        // Convert words
        let words: Vec<TranscriptWord> = api_response
            .words
            .into_iter()
            .map(|w| TranscriptWord {
                word: w.word,
                start: w.start,
                end: w.end,
                confidence: None,
            })
            .collect();

        // Calculate duration from segments if not provided
        let duration_secs = api_response.duration.or_else(|| {
            segments.last().map(|s| s.end)
        });

        let transcription = Transcription {
            text: api_response.text.trim().to_string(),
            language: api_response.language,
            duration_secs,
            segments,
            words,
            speakers: Vec::new(), // Whisper doesn't provide diarization
            usage: TranscriptionUsage {
                audio_seconds: duration_secs.unwrap_or(0.0),
                billable_seconds: duration_secs.unwrap_or(0.0),
            },
        };

        debug!(
            text_length = transcription.text.len(),
            segments = transcription.segments.len(),
            duration = ?transcription.duration_secs,
            "Transcription completed"
        );

        Ok(transcription)
    }

    fn provider_name(&self) -> &str {
        "openai"
    }

    fn model_name(&self) -> &str {
        &self.config.model
    }

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

    fn max_audio_size(&self) -> u64 {
        25 * 1024 * 1024 // 25 MB limit for OpenAI Whisper
    }

    fn supports_diarization(&self) -> bool {
        false // OpenAI Whisper doesn't support speaker diarization
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_provider_creation() {
        let config = TranscriptionConfig {
            provider: "openai".to_string(),
            model: "whisper-1".to_string(),
            api_key: Some("test-key".to_string()),
            ..Default::default()
        };
        let provider = OpenAITranscriptionProvider::new(config);
        assert_eq!(provider.provider_name(), "openai");
        assert_eq!(provider.model_name(), "whisper-1");
    }

    #[test]
    fn test_format_support() {
        let config = TranscriptionConfig::default();
        let provider = OpenAITranscriptionProvider::new(config);

        assert!(provider.supports_format(AudioFormat::Mp3));
        assert!(provider.supports_format(AudioFormat::Wav));
        assert!(provider.supports_format(AudioFormat::Flac));
        assert!(provider.supports_format(AudioFormat::Ogg));
    }

    #[test]
    fn test_max_size() {
        let config = TranscriptionConfig::default();
        let provider = OpenAITranscriptionProvider::new(config);
        assert_eq!(provider.max_audio_size(), 25 * 1024 * 1024);
    }
}
