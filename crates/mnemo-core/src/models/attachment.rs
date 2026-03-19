//! Attachments — binary media files linked to episodes.
//!
//! An [`Attachment`] represents a stored binary file (image, audio, video, or document)
//! that has been uploaded and processed as part of multi-modal memory ingestion.
//! Attachments are always linked to an episode and inherit the user's access controls.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::episode::ProcessingStatus;

/// The modality of an episode's primary content.
///
/// While episodes have always supported text content, this enum extends
/// the model to track the source modality for multi-modal ingestion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Modality {
    /// Text content (default, existing behavior).
    #[default]
    Text,
    /// Image with vision-generated description.
    Image,
    /// Audio with transcript.
    Audio,
    /// Video with key frames and transcript.
    Video,
    /// Document chunk (PDF, Office docs, etc.).
    Document,
}

/// The type of attachment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum AttachmentType {
    /// Image file (JPEG, PNG, GIF, WebP, etc.).
    Image,
    /// Audio file (MP3, WAV, M4A, etc.).
    Audio,
    /// Video file (MP4, WebM, etc.).
    Video,
    /// Document file (PDF, DOCX, PPTX, XLSX, etc.).
    Document,
}

impl AttachmentType {
    /// Infer attachment type from MIME type.
    pub fn from_mime_type(mime: &str) -> Option<Self> {
        let mime_lower = mime.to_lowercase();
        if mime_lower.starts_with("image/") {
            Some(Self::Image)
        } else if mime_lower.starts_with("audio/") {
            Some(Self::Audio)
        } else if mime_lower.starts_with("video/") {
            Some(Self::Video)
        } else if mime_lower.starts_with("application/pdf")
            || mime_lower.starts_with("application/vnd.openxmlformats")
            || mime_lower.starts_with("application/vnd.ms-")
            || mime_lower.starts_with("application/msword")
            || mime_lower.starts_with("text/")
        {
            Some(Self::Document)
        } else {
            None
        }
    }

    /// Convert to corresponding modality.
    pub fn to_modality(self) -> Modality {
        match self {
            Self::Image => Modality::Image,
            Self::Audio => Modality::Audio,
            Self::Video => Modality::Video,
            Self::Document => Modality::Document,
        }
    }
}

/// A binary attachment linked to an episode.
///
/// Attachments store metadata about uploaded files. The actual binary content
/// is stored in a blob store (local filesystem or S3-compatible storage),
/// referenced by `storage_key`.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct Attachment {
    /// Unique identifier for this attachment.
    pub id: Uuid,

    /// The episode this attachment belongs to.
    pub episode_id: Uuid,

    /// The user who owns this attachment (inherited from episode).
    pub user_id: Uuid,

    /// Type of attachment (image, audio, video, document).
    pub attachment_type: AttachmentType,

    /// MIME type (e.g., "image/png", "audio/mp3", "application/pdf").
    pub mime_type: String,

    /// Original filename if provided during upload.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,

    /// Size in bytes.
    pub size_bytes: u64,

    /// Storage location (blob store path/key).
    pub storage_key: String,

    /// Thumbnail storage key (for images/video, pre-signed URL for preview).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thumbnail_key: Option<String>,

    /// Duration in seconds (for audio/video).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_secs: Option<f32>,

    /// Width in pixels (for images/video).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,

    /// Height in pixels (for images/video).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,

    /// Processing status for async vision/transcription.
    pub processing_status: ProcessingStatus,

    /// Error message if processing failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub processing_error: Option<String>,

    /// Vision/transcription model used for processing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub processed_by_model: Option<String>,

    /// Vision-generated description (for images).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// Transcript text (for audio/video).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transcript: Option<String>,

    /// When this attachment was created.
    pub created_at: DateTime<Utc>,
}

impl Attachment {
    /// Create a new attachment with pending processing status.
    pub fn new(
        episode_id: Uuid,
        user_id: Uuid,
        attachment_type: AttachmentType,
        mime_type: String,
        filename: Option<String>,
        size_bytes: u64,
        storage_key: String,
    ) -> Self {
        Self {
            id: Uuid::now_v7(),
            episode_id,
            user_id,
            attachment_type,
            mime_type,
            filename,
            size_bytes,
            storage_key,
            thumbnail_key: None,
            duration_secs: None,
            width: None,
            height: None,
            processing_status: ProcessingStatus::Pending,
            processing_error: None,
            processed_by_model: None,
            description: None,
            transcript: None,
            created_at: Utc::now(),
        }
    }

    /// Mark this attachment as currently being processed.
    pub fn mark_processing(&mut self) {
        self.processing_status = ProcessingStatus::Processing;
    }

    /// Mark this attachment as successfully processed.
    pub fn mark_completed(&mut self, model: &str) {
        self.processing_status = ProcessingStatus::Completed;
        self.processed_by_model = Some(model.to_string());
        self.processing_error = None;
    }

    /// Mark this attachment as failed with an error message.
    pub fn mark_failed(&mut self, error: String) {
        self.processing_status = ProcessingStatus::Failed;
        self.processing_error = Some(error);
    }

    /// Set the vision-generated description (for images).
    pub fn set_description(&mut self, description: String) {
        self.description = Some(description);
    }

    /// Set the transcript (for audio/video).
    pub fn set_transcript(&mut self, transcript: String) {
        self.transcript = Some(transcript);
    }

    /// Set image/video dimensions.
    pub fn set_dimensions(&mut self, width: u32, height: u32) {
        self.width = Some(width);
        self.height = Some(height);
    }

    /// Set audio/video duration.
    pub fn set_duration(&mut self, duration_secs: f32) {
        self.duration_secs = Some(duration_secs);
    }

    /// Set the thumbnail storage key.
    pub fn set_thumbnail(&mut self, thumbnail_key: String) {
        self.thumbnail_key = Some(thumbnail_key);
    }
}

/// Request to upload an attachment.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct UploadAttachmentRequest {
    /// MIME type of the file.
    pub mime_type: String,

    /// Original filename (optional).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,

    /// Size in bytes (for validation before upload).
    pub size_bytes: u64,
}

/// Response after uploading an attachment.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct AttachmentResponse {
    /// The created attachment.
    pub attachment: Attachment,

    /// Pre-signed URL for downloading the original file.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub download_url: Option<String>,

    /// Pre-signed URL for the thumbnail (images/video).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thumbnail_url: Option<String>,
}

/// Query parameters for listing attachments.
#[derive(Debug, Clone, Default, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ListAttachmentsParams {
    /// Filter by attachment type.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attachment_type: Option<AttachmentType>,

    /// Maximum number of results.
    #[serde(default = "default_limit")]
    pub limit: u32,

    /// Pagination cursor (attachment ID to start after).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after: Option<Uuid>,
}

fn default_limit() -> u32 {
    20
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_attachment_type_from_mime() {
        assert_eq!(
            AttachmentType::from_mime_type("image/png"),
            Some(AttachmentType::Image)
        );
        assert_eq!(
            AttachmentType::from_mime_type("image/jpeg"),
            Some(AttachmentType::Image)
        );
        assert_eq!(
            AttachmentType::from_mime_type("audio/mp3"),
            Some(AttachmentType::Audio)
        );
        assert_eq!(
            AttachmentType::from_mime_type("audio/wav"),
            Some(AttachmentType::Audio)
        );
        assert_eq!(
            AttachmentType::from_mime_type("video/mp4"),
            Some(AttachmentType::Video)
        );
        assert_eq!(
            AttachmentType::from_mime_type("application/pdf"),
            Some(AttachmentType::Document)
        );
        assert_eq!(
            AttachmentType::from_mime_type(
                "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
            ),
            Some(AttachmentType::Document)
        );
        assert_eq!(
            AttachmentType::from_mime_type("text/plain"),
            Some(AttachmentType::Document)
        );
        assert_eq!(
            AttachmentType::from_mime_type("application/octet-stream"),
            None
        );
    }

    #[test]
    fn test_attachment_type_to_modality() {
        assert_eq!(AttachmentType::Image.to_modality(), Modality::Image);
        assert_eq!(AttachmentType::Audio.to_modality(), Modality::Audio);
        assert_eq!(AttachmentType::Video.to_modality(), Modality::Video);
        assert_eq!(AttachmentType::Document.to_modality(), Modality::Document);
    }

    #[test]
    fn test_attachment_new() {
        let attachment = Attachment::new(
            Uuid::now_v7(),
            Uuid::now_v7(),
            AttachmentType::Image,
            "image/png".to_string(),
            Some("screenshot.png".to_string()),
            1024 * 1024,
            "user123/attachments/abc123/original.png".to_string(),
        );

        assert_eq!(attachment.attachment_type, AttachmentType::Image);
        assert_eq!(attachment.mime_type, "image/png");
        assert_eq!(attachment.filename, Some("screenshot.png".to_string()));
        assert_eq!(attachment.size_bytes, 1024 * 1024);
        assert_eq!(attachment.processing_status, ProcessingStatus::Pending);
        assert!(attachment.description.is_none());
    }

    #[test]
    fn test_attachment_processing_lifecycle() {
        let mut attachment = Attachment::new(
            Uuid::now_v7(),
            Uuid::now_v7(),
            AttachmentType::Image,
            "image/png".to_string(),
            None,
            1024,
            "test/key".to_string(),
        );

        assert_eq!(attachment.processing_status, ProcessingStatus::Pending);

        attachment.mark_processing();
        assert_eq!(attachment.processing_status, ProcessingStatus::Processing);

        attachment.set_description("A screenshot of a code editor".to_string());
        attachment.set_dimensions(1920, 1080);
        attachment.mark_completed("claude-sonnet-4-20250514");

        assert_eq!(attachment.processing_status, ProcessingStatus::Completed);
        assert_eq!(
            attachment.description,
            Some("A screenshot of a code editor".to_string())
        );
        assert_eq!(attachment.width, Some(1920));
        assert_eq!(attachment.height, Some(1080));
        assert_eq!(
            attachment.processed_by_model,
            Some("claude-sonnet-4-20250514".to_string())
        );
    }

    #[test]
    fn test_attachment_failure() {
        let mut attachment = Attachment::new(
            Uuid::now_v7(),
            Uuid::now_v7(),
            AttachmentType::Audio,
            "audio/mp3".to_string(),
            None,
            1024,
            "test/key".to_string(),
        );

        attachment.mark_processing();
        attachment.mark_failed("Transcription service unavailable".to_string());

        assert_eq!(attachment.processing_status, ProcessingStatus::Failed);
        assert_eq!(
            attachment.processing_error,
            Some("Transcription service unavailable".to_string())
        );
    }

    #[test]
    fn test_attachment_serialization_roundtrip() {
        let attachment = Attachment::new(
            Uuid::now_v7(),
            Uuid::now_v7(),
            AttachmentType::Document,
            "application/pdf".to_string(),
            Some("report.pdf".to_string()),
            5 * 1024 * 1024,
            "user/docs/report.pdf".to_string(),
        );

        let json = serde_json::to_string(&attachment).unwrap();
        let de: Attachment = serde_json::from_str(&json).unwrap();

        assert_eq!(de.id, attachment.id);
        assert_eq!(de.attachment_type, attachment.attachment_type);
        assert_eq!(de.mime_type, attachment.mime_type);
    }

    #[test]
    fn test_modality_default() {
        let modality: Modality = Default::default();
        assert_eq!(modality, Modality::Text);
    }
}
