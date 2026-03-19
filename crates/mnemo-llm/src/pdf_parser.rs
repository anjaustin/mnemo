//! PDF document parser implementation.
//!
//! Uses the `pdf-extract` crate for text extraction from PDF documents.
//! Supports structural chunking based on page boundaries and detected headings.

use tracing::{debug, instrument};

use mnemo_core::error::MnemoError;
use mnemo_core::traits::document::{
    ChunkStrategy, ChunkType, DocumentChunk, DocumentConfig, DocumentFormat, DocumentParser,
    DocumentResult, ParsedDocument,
};

/// PDF document parser.
///
/// Extracts text from PDF documents with page-aware chunking.
/// For complex PDFs with embedded images, use vision models for
/// additional content extraction.
pub struct PdfParser {
    // Configuration for the parser
}

impl PdfParser {
    /// Create a new PDF parser.
    pub fn new() -> Self {
        Self {}
    }
}

impl Default for PdfParser {
    fn default() -> Self {
        Self::new()
    }
}

impl DocumentParser for PdfParser {
    #[instrument(skip(self, data), fields(size = data.len()))]
    async fn parse(
        &self,
        data: &[u8],
        format: DocumentFormat,
        config: &DocumentConfig,
    ) -> DocumentResult<ParsedDocument> {
        if format != DocumentFormat::Pdf {
            return Err(MnemoError::UnsupportedMediaType(format!(
                "PdfParser only supports PDF, got {:?}",
                format
            )));
        }

        // Use pdf-extract to get text
        let text = pdf_extract::extract_text_from_mem(data).map_err(|e| {
            MnemoError::DocumentParsing(format!("Failed to extract text from PDF: {}", e))
        })?;

        debug!(text_length = text.len(), "Extracted text from PDF");

        // Split by pages (pdf-extract uses form feed character \x0c as page separator)
        let pages: Vec<&str> = text.split('\x0c').collect();
        let page_count = pages.len() as u32;

        // Generate chunks based on strategy
        let chunks = match config.chunk_strategy {
            ChunkStrategy::Page => {
                // One chunk per page
                chunk_by_page(&pages)
            }
            ChunkStrategy::Fixed => {
                // Fixed-size chunks with overlap
                chunk_fixed(&text, config.chunk_size, config.chunk_overlap)
            }
            ChunkStrategy::Structural => {
                // Try to detect structure (headings, paragraphs)
                chunk_structural(&pages, config.chunk_size)
            }
        };

        // Try to extract title from first page
        let title = extract_title(&pages);

        // Calculate total char count
        let char_count = text.len();

        Ok(ParsedDocument {
            title,
            author: None, // Would need PDF metadata parsing
            created_at: None,
            page_count,
            char_count,
            chunks,
            images: Vec::new(), // Would need image extraction support
            toc: Vec::new(),    // Would need outline parsing
        })
    }

    fn parser_name(&self) -> &str {
        "pdf-extract"
    }

    fn supports_format(&self, format: DocumentFormat) -> bool {
        format == DocumentFormat::Pdf
    }

    fn max_document_size(&self) -> u64 {
        50 * 1024 * 1024 // 50 MB
    }
}

/// Extract a title from the first page (heuristic).
fn extract_title(pages: &[&str]) -> Option<String> {
    if pages.is_empty() {
        return None;
    }

    let first_page = pages[0].trim();
    if first_page.is_empty() {
        return None;
    }

    // Take the first non-empty line as potential title
    for line in first_page.lines() {
        let line = line.trim();
        if !line.is_empty() && line.len() < 200 {
            // Likely a title if it's not too long
            return Some(line.to_string());
        }
    }

    None
}

/// Chunk by page (one chunk per page).
fn chunk_by_page(pages: &[&str]) -> Vec<DocumentChunk> {
    let mut chunks = Vec::new();
    let mut char_offset = 0;

    for (i, page_text) in pages.iter().enumerate() {
        let text = page_text.trim().to_string();
        if text.is_empty() {
            char_offset += page_text.len() + 1; // +1 for page separator
            continue;
        }

        let char_count = text.len();
        chunks.push(DocumentChunk {
            index: chunks.len() as u32,
            text,
            page: (i + 1) as u32,
            section: None,
            chunk_type: ChunkType::Mixed,
            char_offset,
            char_count,
        });

        char_offset += page_text.len() + 1;
    }

    chunks
}

/// Fixed-size chunks with overlap.
fn chunk_fixed(text: &str, chunk_size: usize, overlap: usize) -> Vec<DocumentChunk> {
    let mut chunks = Vec::new();
    let chars: Vec<char> = text.chars().collect();
    let total_len = chars.len();

    if total_len == 0 {
        return chunks;
    }

    let mut start = 0;
    while start < total_len {
        let end = (start + chunk_size).min(total_len);

        // Try to break at sentence/paragraph boundary
        let mut actual_end = end;
        if end < total_len {
            // Look for sentence end within last 20% of chunk
            let search_start = end.saturating_sub(chunk_size / 5);
            for i in (search_start..end).rev() {
                let c = chars[i];
                if c == '.' || c == '!' || c == '?' || c == '\n' {
                    actual_end = i + 1;
                    break;
                }
            }
        }

        let chunk_text: String = chars[start..actual_end].iter().collect();
        let chunk_text = chunk_text.trim().to_string();

        if !chunk_text.is_empty() {
            chunks.push(DocumentChunk {
                index: chunks.len() as u32,
                text: chunk_text.clone(),
                page: estimate_page(start, text), // Rough estimate
                section: None,
                chunk_type: ChunkType::Paragraph,
                char_offset: start,
                char_count: chunk_text.len(),
            });
        }

        // Move start with overlap
        start = if actual_end >= total_len {
            total_len
        } else {
            actual_end.saturating_sub(overlap)
        };
    }

    chunks
}

/// Structural chunking (detects headings, paragraphs).
fn chunk_structural(pages: &[&str], target_size: usize) -> Vec<DocumentChunk> {
    let mut chunks = Vec::new();
    let mut current_chunk = String::new();
    let mut current_section: Option<String> = None;
    let mut chunk_start_page = 1u32;
    let mut char_offset = 0usize;
    let mut chunk_start_offset = 0usize;

    for (page_idx, page_text) in pages.iter().enumerate() {
        let page_num = (page_idx + 1) as u32;

        for line in page_text.lines() {
            let line = line.trim();
            if line.is_empty() {
                current_chunk.push('\n');
                continue;
            }

            // Detect headings (heuristic: short lines, possibly all caps or numbered)
            let is_heading = is_likely_heading(line);

            if is_heading {
                // Flush current chunk if substantial
                if current_chunk.trim().len() > 50 {
                    let text = current_chunk.trim().to_string();
                    chunks.push(DocumentChunk {
                        index: chunks.len() as u32,
                        text: text.clone(),
                        page: chunk_start_page,
                        section: current_section.clone(),
                        chunk_type: ChunkType::Paragraph,
                        char_offset: chunk_start_offset,
                        char_count: text.len(),
                    });
                }

                // Start new section
                current_section = Some(line.to_string());
                current_chunk = String::new();
                chunk_start_page = page_num;
                chunk_start_offset = char_offset;
            }

            current_chunk.push_str(line);
            current_chunk.push('\n');

            // If chunk is getting large, flush it
            if current_chunk.len() > target_size {
                let text = current_chunk.trim().to_string();
                if !text.is_empty() {
                    chunks.push(DocumentChunk {
                        index: chunks.len() as u32,
                        text: text.clone(),
                        page: chunk_start_page,
                        section: current_section.clone(),
                        chunk_type: ChunkType::Paragraph,
                        char_offset: chunk_start_offset,
                        char_count: text.len(),
                    });
                }
                current_chunk = String::new();
                chunk_start_page = page_num;
                chunk_start_offset = char_offset + line.len();
            }

            char_offset += line.len() + 1;
        }

        char_offset += 1; // Page separator
    }

    // Flush remaining content
    let text = current_chunk.trim().to_string();
    if !text.is_empty() {
        chunks.push(DocumentChunk {
            index: chunks.len() as u32,
            text: text.clone(),
            page: chunk_start_page,
            section: current_section,
            chunk_type: ChunkType::Paragraph,
            char_offset: chunk_start_offset,
            char_count: text.len(),
        });
    }

    chunks
}

/// Heuristic to detect if a line is likely a heading.
fn is_likely_heading(line: &str) -> bool {
    let line = line.trim();

    // Too short or too long
    if line.len() < 3 || line.len() > 100 {
        return false;
    }

    // Numbered heading (e.g., "1. Introduction", "Chapter 1")
    if line.starts_with(|c: char| c.is_ascii_digit())
        && (line.contains('.') || line.to_lowercase().contains("chapter"))
    {
        return true;
    }

    // All caps (common for headings)
    if line
        .chars()
        .filter(|c| c.is_alphabetic())
        .all(|c| c.is_uppercase())
    {
        return line.len() > 3 && line.len() < 60;
    }

    // Title case with short length (potential section header)
    let words: Vec<&str> = line.split_whitespace().collect();
    if words.len() <= 6 {
        let capitalized = words
            .iter()
            .filter(|w| w.len() > 1)
            .filter(|w| w.chars().next().map_or(false, |c| c.is_uppercase()))
            .count();
        if capitalized >= words.len().saturating_sub(1) && words.len() >= 2 {
            return true;
        }
    }

    false
}

/// Estimate page number from character offset (rough).
fn estimate_page(char_offset: usize, text: &str) -> u32 {
    // Count form feeds before this offset
    let page_count = text[..char_offset.min(text.len())]
        .chars()
        .filter(|c| *c == '\x0c')
        .count();
    (page_count + 1) as u32
}

/// Simple text document parser.
///
/// Handles plain text, Markdown, and HTML documents.
pub struct TextDocumentParser;

impl TextDocumentParser {
    pub fn new() -> Self {
        Self
    }
}

impl Default for TextDocumentParser {
    fn default() -> Self {
        Self::new()
    }
}

impl DocumentParser for TextDocumentParser {
    #[instrument(skip(self, data), fields(size = data.len()))]
    async fn parse(
        &self,
        data: &[u8],
        format: DocumentFormat,
        config: &DocumentConfig,
    ) -> DocumentResult<ParsedDocument> {
        // Convert bytes to string
        let text = String::from_utf8_lossy(data).to_string();

        // For HTML, strip tags (simple approach)
        let text = match format {
            DocumentFormat::Html => strip_html_tags(&text),
            _ => text,
        };

        let char_count = text.len();

        // Generate chunks
        let chunks = match config.chunk_strategy {
            ChunkStrategy::Fixed => chunk_fixed(&text, config.chunk_size, config.chunk_overlap),
            _ => chunk_structural(&[text.as_str()], config.chunk_size),
        };

        // Try to extract title (first heading for markdown, first line for text)
        let title = extract_text_title(&text, format);

        Ok(ParsedDocument {
            title,
            author: None,
            created_at: None,
            page_count: 1,
            char_count,
            chunks,
            images: Vec::new(),
            toc: Vec::new(),
        })
    }

    fn parser_name(&self) -> &str {
        "text-parser"
    }

    fn supports_format(&self, format: DocumentFormat) -> bool {
        matches!(
            format,
            DocumentFormat::Txt | DocumentFormat::Markdown | DocumentFormat::Html
        )
    }
}

/// Strip HTML tags (simple regex-free approach).
fn strip_html_tags(html: &str) -> String {
    let mut result = String::with_capacity(html.len());
    let mut in_tag = false;

    for c in html.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => result.push(c),
            _ => {}
        }
    }

    // Decode common HTML entities
    result
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
}

/// Extract title from text document.
fn extract_text_title(text: &str, format: DocumentFormat) -> Option<String> {
    let first_line = text.lines().next()?.trim();

    match format {
        DocumentFormat::Markdown => {
            // Look for # heading
            if first_line.starts_with('#') {
                return Some(first_line.trim_start_matches('#').trim().to_string());
            }
        }
        _ => {}
    }

    // Use first non-empty line if short enough
    if !first_line.is_empty() && first_line.len() < 200 {
        Some(first_line.to_string())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_likely_heading() {
        assert!(is_likely_heading("INTRODUCTION"));
        assert!(is_likely_heading("1. Introduction"));
        assert!(is_likely_heading("Chapter 1"));
        assert!(is_likely_heading("The Quick Brown Fox")); // Title case
        assert!(!is_likely_heading(
            "This is a regular sentence with many words."
        ));
        assert!(!is_likely_heading("a")); // Too short
    }

    #[test]
    fn test_strip_html_tags() {
        assert_eq!(strip_html_tags("<p>Hello</p>"), "Hello");
        assert_eq!(strip_html_tags("<b>Bold</b> text"), "Bold text");
        assert_eq!(strip_html_tags("No &amp; tags"), "No & tags");
    }

    #[test]
    fn test_chunk_fixed() {
        let text = "Hello world. This is a test. Another sentence here.";
        let chunks = chunk_fixed(text, 20, 5);
        assert!(!chunks.is_empty());
        // All text should be covered
        let total: usize = chunks.iter().map(|c| c.char_count).sum();
        assert!(total > 0);
    }

    #[tokio::test]
    async fn test_text_parser() {
        let parser = TextDocumentParser::new();
        let data = b"# Hello World\n\nThis is a test document.\n\nWith multiple paragraphs.";
        let config = DocumentConfig::default();

        let result = parser.parse(data, DocumentFormat::Markdown, &config).await;
        assert!(result.is_ok());

        let doc = result.unwrap();
        assert_eq!(doc.title, Some("Hello World".to_string()));
        assert!(!doc.chunks.is_empty());
    }
}
