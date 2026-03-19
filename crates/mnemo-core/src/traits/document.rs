//! Document parsing trait for extracting text and structure from documents.
//!
//! The [`DocumentParser`] trait defines the interface for parsing documents
//! (PDF, DOCX, PPTX, XLSX, etc.) and extracting text with structural information
//! for memory ingestion.

use serde::{Deserialize, Serialize};

use crate::error::MnemoError;

pub type DocumentResult<T> = Result<T, MnemoError>;

/// Configuration for document parsing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentConfig {
    /// Enable document parsing.
    #[serde(default)]
    pub enabled: bool,
    /// Maximum file size in bytes.
    #[serde(default = "default_max_size")]
    pub max_size_bytes: u64,
    /// Chunking strategy: "structural" or "fixed".
    #[serde(default = "default_chunk_strategy")]
    pub chunk_strategy: ChunkStrategy,
    /// Target chunk size in characters (for fixed chunking).
    #[serde(default = "default_chunk_size")]
    pub chunk_size: usize,
    /// Overlap between chunks in characters.
    #[serde(default = "default_chunk_overlap")]
    pub chunk_overlap: usize,
    /// Extract embedded images for vision processing.
    #[serde(default)]
    pub extract_images: bool,
    /// Process documents synchronously (wait for parsing) or async (background).
    #[serde(default)]
    pub sync_processing: bool,
}

fn default_max_size() -> u64 {
    50 * 1024 * 1024 // 50 MB
}

fn default_chunk_strategy() -> ChunkStrategy {
    ChunkStrategy::Structural
}

fn default_chunk_size() -> usize {
    1500 // ~375 tokens
}

fn default_chunk_overlap() -> usize {
    200
}

impl Default for DocumentConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            max_size_bytes: default_max_size(),
            chunk_strategy: default_chunk_strategy(),
            chunk_size: default_chunk_size(),
            chunk_overlap: default_chunk_overlap(),
            extract_images: false,
            sync_processing: false,
        }
    }
}

/// Chunking strategy for documents.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChunkStrategy {
    /// Chunk by document structure (headings, paragraphs, sections).
    #[default]
    Structural,
    /// Fixed-size chunks with overlap.
    Fixed,
    /// One chunk per page.
    Page,
}

/// Result of parsing a document.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParsedDocument {
    /// Document title (from metadata or first heading).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,

    /// Document author (from metadata).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,

    /// Creation date (from metadata).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,

    /// Total number of pages.
    pub page_count: u32,

    /// Total character count.
    pub char_count: usize,

    /// Document chunks for embedding.
    pub chunks: Vec<DocumentChunk>,

    /// Embedded images (if extract_images enabled).
    #[serde(default)]
    pub images: Vec<EmbeddedImage>,

    /// Table of contents (if available).
    #[serde(default)]
    pub toc: Vec<TocEntry>,
}

/// A chunk of document text with location info.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentChunk {
    /// Chunk index (0-based).
    pub index: u32,

    /// Text content of this chunk.
    pub text: String,

    /// Page number (1-based) where this chunk starts.
    pub page: u32,

    /// Section heading (if available).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub section: Option<String>,

    /// Chunk type.
    pub chunk_type: ChunkType,

    /// Character offset in the original document.
    pub char_offset: usize,

    /// Character count.
    pub char_count: usize,
}

/// Type of document chunk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChunkType {
    /// Title or main heading.
    Title,
    /// Section heading.
    Heading,
    /// Regular paragraph text.
    Paragraph,
    /// Table content.
    Table,
    /// List content.
    List,
    /// Code block.
    Code,
    /// Footnote or endnote.
    Footnote,
    /// Caption (for figures/tables).
    Caption,
    /// Mixed or unknown content.
    Mixed,
}

/// An embedded image extracted from a document.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddedImage {
    /// Image index (0-based).
    pub index: u32,

    /// Page number where the image appears.
    pub page: u32,

    /// Image data (PNG/JPEG bytes).
    pub data: Vec<u8>,

    /// MIME type (e.g., "image/png").
    pub mime_type: String,

    /// Width in pixels.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,

    /// Height in pixels.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,

    /// Alt text or caption (if available).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alt_text: Option<String>,
}

/// Table of contents entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TocEntry {
    /// Heading text.
    pub text: String,

    /// Heading level (1 = H1, 2 = H2, etc.).
    pub level: u8,

    /// Page number (1-based).
    pub page: u32,
}

/// Supported document formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DocumentFormat {
    /// PDF document.
    Pdf,
    /// Microsoft Word document (.docx).
    Docx,
    /// Microsoft Word legacy (.doc).
    Doc,
    /// Microsoft PowerPoint (.pptx).
    Pptx,
    /// Microsoft Excel (.xlsx).
    Xlsx,
    /// Rich Text Format.
    Rtf,
    /// Plain text.
    Txt,
    /// Markdown.
    Markdown,
    /// HTML document.
    Html,
    /// EPUB ebook.
    Epub,
}

impl DocumentFormat {
    /// Get the MIME type for this format.
    pub fn media_type(&self) -> &str {
        match self {
            Self::Pdf => "application/pdf",
            Self::Docx => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            Self::Doc => "application/msword",
            Self::Pptx => {
                "application/vnd.openxmlformats-officedocument.presentationml.presentation"
            }
            Self::Xlsx => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
            Self::Rtf => "application/rtf",
            Self::Txt => "text/plain",
            Self::Markdown => "text/markdown",
            Self::Html => "text/html",
            Self::Epub => "application/epub+zip",
        }
    }

    /// Parse document format from MIME type.
    pub fn from_mime(mime: &str) -> Option<Self> {
        let mime_lower = mime.to_lowercase();
        match mime_lower.as_str() {
            "application/pdf" => Some(Self::Pdf),
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document" => {
                Some(Self::Docx)
            }
            "application/msword" => Some(Self::Doc),
            "application/vnd.openxmlformats-officedocument.presentationml.presentation" => {
                Some(Self::Pptx)
            }
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet" => Some(Self::Xlsx),
            "application/rtf" | "text/rtf" => Some(Self::Rtf),
            "text/plain" => Some(Self::Txt),
            "text/markdown" | "text/x-markdown" => Some(Self::Markdown),
            "text/html" | "application/xhtml+xml" => Some(Self::Html),
            "application/epub+zip" => Some(Self::Epub),
            _ => None,
        }
    }

    /// Get the file extension for this format.
    pub fn extension(&self) -> &str {
        match self {
            Self::Pdf => "pdf",
            Self::Docx => "docx",
            Self::Doc => "doc",
            Self::Pptx => "pptx",
            Self::Xlsx => "xlsx",
            Self::Rtf => "rtf",
            Self::Txt => "txt",
            Self::Markdown => "md",
            Self::Html => "html",
            Self::Epub => "epub",
        }
    }
}

/// Trait for document parsers.
///
/// Implement this trait to add support for different document formats
/// (PDF, DOCX, etc.).
#[allow(async_fn_in_trait)]
pub trait DocumentParser: Send + Sync {
    /// Parse a document and extract text with structure.
    ///
    /// # Arguments
    /// * `data` - Raw document bytes.
    /// * `format` - The document format.
    /// * `config` - Parsing configuration.
    ///
    /// # Returns
    /// A `ParsedDocument` containing chunks and metadata.
    async fn parse(
        &self,
        data: &[u8],
        format: DocumentFormat,
        config: &DocumentConfig,
    ) -> DocumentResult<ParsedDocument>;

    /// Get the parser name for logging/debugging.
    fn parser_name(&self) -> &str;

    /// Check if the parser supports a given document format.
    fn supports_format(&self, format: DocumentFormat) -> bool;

    /// Get the maximum supported document size in bytes.
    fn max_document_size(&self) -> u64 {
        50 * 1024 * 1024 // 50 MB default
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_document_format_from_mime() {
        assert_eq!(
            DocumentFormat::from_mime("application/pdf"),
            Some(DocumentFormat::Pdf)
        );
        assert_eq!(
            DocumentFormat::from_mime(
                "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
            ),
            Some(DocumentFormat::Docx)
        );
        assert_eq!(
            DocumentFormat::from_mime("text/plain"),
            Some(DocumentFormat::Txt)
        );
        assert_eq!(
            DocumentFormat::from_mime("text/markdown"),
            Some(DocumentFormat::Markdown)
        );
        assert_eq!(DocumentFormat::from_mime("video/mp4"), None);
    }

    #[test]
    fn test_document_format_media_type() {
        assert_eq!(DocumentFormat::Pdf.media_type(), "application/pdf");
        assert_eq!(DocumentFormat::Txt.media_type(), "text/plain");
    }

    #[test]
    fn test_chunk_strategy_default() {
        assert_eq!(ChunkStrategy::default(), ChunkStrategy::Structural);
    }

    #[test]
    fn test_document_config_default() {
        let config = DocumentConfig::default();
        assert!(!config.enabled);
        assert_eq!(config.max_size_bytes, 50 * 1024 * 1024);
        assert_eq!(config.chunk_strategy, ChunkStrategy::Structural);
        assert_eq!(config.chunk_size, 1500);
        assert_eq!(config.chunk_overlap, 200);
    }
}
