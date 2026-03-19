# Multi-Modal Memory PRD

**Status**: Draft  
**Target Version**: v0.11.0  
**Priority**: P0 — Competitive parity gap

---

## Executive Summary

Mnemo currently supports **text-only** memory ingestion. This is a documented competitive gap:

| Vendor | Multi-Modal Support |
|--------|---------------------|
| Mem0 | ✅ Images (JPG, PNG), Documents (MDX, TXT), PDFs |
| Zep | ⚠️ Partial (document loaders) |
| Letta | ⚠️ Document processing via code interpreter |
| **Mnemo** | ❌ Text only |

Multi-modal memory enables:
- **Image memory**: Screenshots, photos, diagrams, charts referenced in conversations
- **Audio memory**: Voice memos, meeting recordings, podcasts
- **Document memory**: PDFs, presentations, spreadsheets with structure preservation
- **Video memory**: Key frames, transcripts, scene descriptions

This PRD defines the architecture for adding multi-modal support while preserving Mnemo's differentiators (temporal reasoning, graph construction, governance).

---

## Problem Statement

### Current Limitations

1. **Episode content is text-only**: `content: String` field cannot hold binary data
2. **Embeddings are text-only**: `EmbeddingProvider::embed(&str)` only accepts text
3. **No blob storage**: No infrastructure for storing images, audio, video files
4. **No vision/audio model integration**: LLM providers don't support multi-modal inputs
5. **Entity extraction is text-based**: Cannot extract entities from images or transcripts

### User Impact

- Agents cannot remember what was shown in an image
- Meeting recordings cannot be ingested as memory
- PDF attachments are ignored
- Diagrams and screenshots are lost context

---

## Goals

### Primary Goals

1. **Image memory**: Ingest images, extract descriptions, embed for retrieval
2. **Document memory**: Ingest PDFs/Office docs, extract text + structure, embed
3. **Audio memory**: Ingest audio files, transcribe, embed transcript
4. **Unified retrieval**: Query across text AND multi-modal memories

### Secondary Goals

1. **Video memory**: Key frame extraction + transcript + scene descriptions
2. **Multi-modal entity extraction**: Identify people, products, logos in images
3. **Cross-modal linking**: Connect image of "Product X" to text mentions of "Product X"

### Non-Goals (v0.11.0)

1. Real-time video/audio streaming
2. Image/audio generation (we're memory, not generation)
3. OCR as primary text extraction (use vision models)
4. Storing raw video files (store key frames + transcript)

---

## Architecture

### High-Level Design

```
                                    ┌─────────────────────┐
                                    │    Blob Storage     │
                                    │  (S3/MinIO/Local)   │
                                    └──────────┬──────────┘
                                               │
┌─────────────────┐    ┌─────────────────┐    │    ┌─────────────────┐
│  Image Upload   │───▶│  Multi-Modal    │────┼───▶│   Episode       │
│  Audio Upload   │    │  Processor      │    │    │   (enhanced)    │
│  Document Upload│    │                 │    │    │                 │
└─────────────────┘    │  - Vision LLM   │    │    │  content: text  │
                       │  - Transcriber  │    │    │  attachments: []│
                       │  - Doc Parser   │    │    │  modality: ...  │
                       └────────┬────────┘    │    └────────┬────────┘
                                │             │             │
                                ▼             │             ▼
                       ┌─────────────────┐    │    ┌─────────────────┐
                       │  Multi-Modal    │    │    │  Text Embedding │
                       │  Embedding      │────┘    │  (existing)     │
                       │  (CLIP, etc.)   │         └────────┬────────┘
                       └────────┬────────┘                  │
                                │                           │
                                ▼                           ▼
                       ┌─────────────────────────────────────────────┐
                       │              Qdrant                          │
                       │  - text_embeddings collection (existing)     │
                       │  - image_embeddings collection (new)         │
                       │  - audio_embeddings collection (new)         │
                       └─────────────────────────────────────────────┘
```

### Processing Pipelines

#### Image Pipeline

```
Image Upload
    │
    ▼
┌─────────────────────────────────────────────────────────┐
│ 1. Store original in blob storage                       │
│ 2. Generate thumbnail (for dashboard preview)           │
│ 3. Vision LLM description (GPT-4V / Claude Vision)      │
│    - Scene description                                   │
│    - Text extraction (signs, labels, UI)                 │
│    - Entity identification (people, products, logos)     │
│ 4. CLIP embedding (for image-to-image similarity)       │
│ 5. Text embedding of description (for text-to-image)    │
│ 6. Create Episode with modality=image                   │
│ 7. Entity extraction from description                   │
│ 8. Graph construction (normal pipeline)                 │
└─────────────────────────────────────────────────────────┘
```

#### Audio Pipeline

```
Audio Upload
    │
    ▼
┌─────────────────────────────────────────────────────────┐
│ 1. Store original in blob storage                       │
│ 2. Transcribe (Whisper / Deepgram / AssemblyAI)        │
│    - With speaker diarization if available              │
│    - With timestamps                                     │
│ 3. Text embedding of transcript                         │
│ 4. Create Episode with modality=audio                   │
│    - content = transcript                                │
│    - attachments = [audio_ref]                          │
│ 5. Entity extraction from transcript                    │
│ 6. Graph construction (normal pipeline)                 │
└─────────────────────────────────────────────────────────┘
```

#### Document Pipeline

```
Document Upload (PDF, DOCX, PPTX, XLSX)
    │
    ▼
┌─────────────────────────────────────────────────────────┐
│ 1. Store original in blob storage                       │
│ 2. Parse document structure                             │
│    - PDF: pdfium / pdf-extract                          │
│    - Office: calamine / docx-rs                         │
│ 3. Extract text with structure preservation             │
│    - Headings, paragraphs, tables, lists                │
│    - Page boundaries                                     │
│ 4. Extract embedded images → Image Pipeline             │
│ 5. Chunk by structure (not arbitrary token windows)     │
│ 6. Text embedding per chunk                             │
│ 7. Create Episode per chunk with modality=document      │
│    - Preserve document_id for cross-chunk linking       │
│ 8. Entity extraction from chunks                        │
│ 9. Graph construction                                   │
└─────────────────────────────────────────────────────────┘
```

---

## Data Model

### New Types

```rust
/// Modality of an episode's primary content.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Modality {
    /// Text content (existing behavior)
    Text,
    /// Image with vision-generated description
    Image,
    /// Audio with transcript
    Audio,
    /// Video with key frames + transcript
    Video,
    /// Document chunk (PDF, Office, etc.)
    Document,
}

/// Reference to a stored blob (image, audio, video, document).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Attachment {
    pub id: Uuid,
    pub episode_id: Uuid,
    pub user_id: Uuid,
    
    /// Type of attachment
    pub attachment_type: AttachmentType,
    
    /// MIME type (e.g., "image/png", "audio/mp3", "application/pdf")
    pub mime_type: String,
    
    /// Original filename if provided
    pub filename: Option<String>,
    
    /// Size in bytes
    pub size_bytes: u64,
    
    /// Storage location (blob store path/key)
    pub storage_key: String,
    
    /// Thumbnail storage key (for images/video)
    pub thumbnail_key: Option<String>,
    
    /// Duration in seconds (for audio/video)
    pub duration_secs: Option<f32>,
    
    /// Dimensions (for images/video)
    pub width: Option<u32>,
    pub height: Option<u32>,
    
    /// Processing status
    pub processing_status: ProcessingStatus,
    
    /// Vision/transcription model used
    pub processed_by_model: Option<String>,
    
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttachmentType {
    Image,
    Audio,
    Video,
    Document,
}

/// Configuration for multi-modal processing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MultiModalConfig {
    /// Enable multi-modal processing
    pub enabled: bool,
    
    /// Vision model configuration
    pub vision: Option<VisionConfig>,
    
    /// Transcription configuration
    pub transcription: Option<TranscriptionConfig>,
    
    /// Image embedding configuration (CLIP, etc.)
    pub image_embedding: Option<ImageEmbeddingConfig>,
    
    /// Blob storage configuration
    pub blob_storage: BlobStorageConfig,
    
    /// Max file sizes by type
    pub max_image_size_mb: u32,
    pub max_audio_size_mb: u32,
    pub max_document_size_mb: u32,
    
    /// Allowed MIME types
    pub allowed_image_types: Vec<String>,
    pub allowed_audio_types: Vec<String>,
    pub allowed_document_types: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VisionConfig {
    /// Provider: "anthropic", "openai", "ollama"
    pub provider: String,
    /// Model: "claude-sonnet-4-20250514", "gpt-4o", "llava"
    pub model: String,
    pub api_key: Option<String>,
    pub base_url: Option<String>,
    /// Max tokens for vision response
    pub max_tokens: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscriptionConfig {
    /// Provider: "openai", "deepgram", "assemblyai", "local"
    pub provider: String,
    /// Model: "whisper-1", "nova-2", etc.
    pub model: String,
    pub api_key: Option<String>,
    /// Enable speaker diarization
    pub diarization: bool,
    /// Language hint (ISO 639-1)
    pub language: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageEmbeddingConfig {
    /// Provider: "openai", "clip-local"
    pub provider: String,
    /// Model: "clip-vit-base-patch32"
    pub model: String,
    pub dimensions: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum BlobStorageConfig {
    /// Local filesystem storage
    Local { path: String },
    /// S3-compatible storage (S3, MinIO, R2, etc.)
    S3 {
        bucket: String,
        region: Option<String>,
        endpoint: Option<String>,
        access_key_id: Option<String>,
        secret_access_key: Option<String>,
    },
}
```

### Modified Types

```rust
// Episode gains modality and attachments
pub struct Episode {
    // ... existing fields ...
    
    /// Primary modality of this episode
    #[serde(default)]
    pub modality: Modality,
    
    /// Attached media (images, audio, documents)
    #[serde(default)]
    pub attachment_ids: Vec<Uuid>,
    
    /// For document chunks: the parent document ID
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_document_id: Option<Uuid>,
    
    /// For document chunks: page number or section reference
    #[serde(skip_serializing_if = "Option::is_none")]
    pub document_location: Option<String>,
}
```

### Storage Schema

**Redis Keys:**
```
{prefix}attachment:{id}              # Attachment metadata (JSON)
{prefix}attachments:user:{user_id}   # User's attachments (sorted set by created_at)
{prefix}attachments:episode:{ep_id}  # Episode's attachments (set)
{prefix}document:{doc_id}:chunks     # Document's chunk episode IDs (list)
```

**Qdrant Collections:**
```
{prefix}_image_embeddings    # CLIP embeddings for image-to-image search
{prefix}_entities            # (existing) includes image-derived entities
{prefix}_episodes            # (existing) includes all modalities
```

**Blob Storage Structure:**
```
/{user_id}/attachments/{attachment_id}/original.{ext}
/{user_id}/attachments/{attachment_id}/thumbnail.jpg
/{user_id}/attachments/{attachment_id}/processed.json  # Vision/transcription output
```

---

## API Design

### Upload Endpoints

```
# Upload image and create episode
POST /api/v1/sessions/:session_id/episodes/image
Content-Type: multipart/form-data
- file: <binary>
- metadata: <json> (optional)

Response:
{
  "episode": { ... },
  "attachment": { ... },
  "description": "A screenshot of a Slack conversation showing..."
}

# Upload audio and create episode
POST /api/v1/sessions/:session_id/episodes/audio
Content-Type: multipart/form-data
- file: <binary>
- metadata: <json> (optional)
- language: "en" (optional)
- diarization: true (optional)

Response:
{
  "episode": { ... },
  "attachment": { ... },
  "transcript": "Speaker 1: Hello, welcome to...",
  "segments": [
    { "speaker": "Speaker 1", "start": 0.0, "end": 2.5, "text": "Hello, welcome to..." }
  ]
}

# Upload document
POST /api/v1/sessions/:session_id/episodes/document
Content-Type: multipart/form-data
- file: <binary>
- metadata: <json> (optional)
- chunk_strategy: "structural" | "fixed" (optional, default: structural)

Response:
{
  "document_id": "...",
  "chunks": [
    { "episode": { ... }, "page": 1, "section": "Introduction" },
    ...
  ],
  "attachment": { ... }
}

# Attach media to existing episode
POST /api/v1/episodes/:episode_id/attachments
Content-Type: multipart/form-data
- file: <binary>
- type: "image" | "audio" | "document"

Response:
{
  "attachment": { ... }
}
```

### Retrieval Endpoints

```
# Context retrieval (enhanced)
POST /api/v1/users/:user_id/context
{
  "query": "What did the product mockup look like?",
  "include_modalities": ["text", "image", "audio", "document"],  # NEW
  "image_query": "<base64 image>",  # NEW: image-to-image search
  "max_tokens": 4000
}

Response:
{
  "context": "...",
  "sources": [
    {
      "episode_id": "...",
      "modality": "image",
      "attachment_id": "...",
      "thumbnail_url": "...",  # Pre-signed URL
      "description": "A mockup showing...",
      "relevance_score": 0.87
    },
    ...
  ]
}

# Get attachment
GET /api/v1/attachments/:attachment_id

Response:
{
  "attachment": { ... },
  "download_url": "...",  # Pre-signed URL with expiry
  "thumbnail_url": "..."
}

# Get attachment content (redirect to blob storage)
GET /api/v1/attachments/:attachment_id/content
→ 302 Redirect to pre-signed blob URL

# List attachments for user
GET /api/v1/users/:user_id/attachments?type=image&limit=20&offset=0
```

### SDK Extensions

**Python:**
```python
from mnemo import MnemoClient
from pathlib import Path

client = MnemoClient()

# Upload image
episode = client.add_image(
    session_id="...",
    image=Path("screenshot.png"),  # or bytes
    metadata={"source": "slack"}
)
print(episode.description)  # Vision-generated description

# Upload audio
episode = client.add_audio(
    session_id="...",
    audio=Path("meeting.mp3"),
    diarization=True
)
print(episode.transcript)

# Upload document
doc = client.add_document(
    session_id="...",
    document=Path("report.pdf")
)
for chunk in doc.chunks:
    print(f"Page {chunk.page}: {chunk.content[:100]}...")

# Multi-modal context retrieval
context = client.context(
    user_id="...",
    query="What was discussed in the meeting about the product?",
    include_modalities=["text", "audio"]
)
for source in context.sources:
    if source.modality == "audio":
        print(f"From audio: {source.transcript_excerpt}")
```

**TypeScript:**
```typescript
import { MnemoClient } from '@mnemo/client';
import { readFileSync } from 'fs';

const client = new MnemoClient();

// Upload image
const episode = await client.addImage({
  sessionId: '...',
  image: readFileSync('screenshot.png'),
  mimeType: 'image/png'
});
console.log(episode.description);

// Multi-modal context
const context = await client.context({
  userId: '...',
  query: 'What did the dashboard look like?',
  includeModalities: ['text', 'image']
});
```

---

## Provider Integrations

### Vision Models

| Provider | Models | Features |
|----------|--------|----------|
| Anthropic | claude-sonnet-4-20250514, claude-3-5-sonnet | Best for detailed descriptions, entity extraction |
| OpenAI | gpt-4o, gpt-4o-mini | Good balance of speed/quality |
| Ollama | llava, bakllava | Local, privacy-preserving |

### Transcription

| Provider | Models | Features |
|----------|--------|----------|
| OpenAI | whisper-1 | Best accuracy, no diarization |
| Deepgram | nova-2 | Fast, good diarization |
| AssemblyAI | best, nano | Speaker labels, sentiment |
| Local | faster-whisper | Privacy, GPU required |

### Image Embeddings

| Provider | Models | Dimensions | Notes |
|----------|--------|------------|-------|
| OpenAI | clip-vit-large | 768 | Best quality |
| Local | clip-ViT-B-32 | 512 | Via `rust-bert` or ONNX |

---

## Implementation Plan

### Phase 1: Foundation (2 weeks)

- [ ] `Modality` enum and `Attachment` model
- [ ] `BlobStore` trait with Local and S3 implementations
- [ ] `AttachmentStore` trait for Redis metadata
- [ ] Blob upload/download endpoints
- [ ] Pre-signed URL generation
- [ ] Episode model extension (modality, attachment_ids)
- [ ] Unit tests for blob storage

### Phase 2: Image Support (2 weeks)

- [ ] `VisionProvider` trait
- [ ] Anthropic Vision implementation
- [ ] OpenAI Vision implementation
- [ ] Image upload endpoint with vision processing
- [ ] Thumbnail generation
- [ ] CLIP embedding integration (optional)
- [ ] Image-to-image search
- [ ] Entity extraction from image descriptions
- [ ] Integration tests

### Phase 3: Audio Support (2 weeks)

- [ ] `TranscriptionProvider` trait
- [ ] OpenAI Whisper implementation
- [ ] Deepgram implementation (optional)
- [ ] Audio upload endpoint with transcription
- [ ] Speaker diarization support
- [ ] Timestamp-aligned transcript storage
- [ ] Entity extraction from transcripts
- [ ] Integration tests

### Phase 4: Document Support (2 weeks)

- [ ] Document parser (PDF, DOCX, PPTX, XLSX)
- [ ] Structure-aware chunking
- [ ] Embedded image extraction → Image Pipeline
- [ ] Document upload endpoint
- [ ] Cross-chunk document linking
- [ ] Table extraction and embedding
- [ ] Integration tests

### Phase 5: Retrieval & SDK (1 week)

- [ ] Multi-modal context retrieval
- [ ] Modality filtering in queries
- [ ] Python SDK extensions
- [ ] TypeScript SDK extensions
- [ ] Dashboard attachment preview
- [ ] Documentation

---

## Configuration

### Environment Variables

```bash
# Enable multi-modal
MNEMO_MULTIMODAL_ENABLED=true

# Blob storage
MNEMO_BLOB_STORAGE_TYPE=s3  # or "local"
MNEMO_BLOB_STORAGE_PATH=/var/mnemo/blobs  # for local
MNEMO_BLOB_S3_BUCKET=mnemo-attachments
MNEMO_BLOB_S3_REGION=us-east-1
MNEMO_BLOB_S3_ENDPOINT=  # for MinIO/R2
AWS_ACCESS_KEY_ID=...
AWS_SECRET_ACCESS_KEY=...

# Vision
MNEMO_VISION_PROVIDER=anthropic
MNEMO_VISION_MODEL=claude-sonnet-4-20250514
MNEMO_VISION_API_KEY=...  # defaults to ANTHROPIC_API_KEY

# Transcription
MNEMO_TRANSCRIPTION_PROVIDER=openai
MNEMO_TRANSCRIPTION_MODEL=whisper-1
MNEMO_TRANSCRIPTION_API_KEY=...  # defaults to OPENAI_API_KEY
MNEMO_TRANSCRIPTION_DIARIZATION=true

# Image embeddings (optional)
MNEMO_IMAGE_EMBEDDING_PROVIDER=openai
MNEMO_IMAGE_EMBEDDING_MODEL=clip-vit-large
MNEMO_IMAGE_EMBEDDING_DIMENSIONS=768

# Limits
MNEMO_MAX_IMAGE_SIZE_MB=10
MNEMO_MAX_AUDIO_SIZE_MB=100
MNEMO_MAX_DOCUMENT_SIZE_MB=50
```

---

## Security Considerations

### Access Control

1. **Attachment isolation**: Attachments inherit user_id from episode; access control enforced
2. **Pre-signed URLs**: Short expiry (15 min default), user-scoped
3. **Blob storage security**: S3 bucket policies, no public access
4. **Content validation**: MIME type verification, magic byte checking
5. **Size limits**: Configurable per-type limits to prevent abuse

### Data Classification

1. Attachments inherit episode's classification (public/internal/confidential/restricted)
2. Vision/transcription processing respects classification (no external API for restricted)
3. Thumbnail generation for restricted content uses local processing only

### Compliance

1. **GDPR**: Attachment deletion cascades from episode/user deletion
2. **Retention**: Attachments follow episode retention policies
3. **Audit**: Attachment access logged in governance audit trail
4. **Encryption**: Blob storage encryption at rest (S3 SSE or local disk encryption)

---

## Success Metrics

| Metric | Target | Measurement |
|--------|--------|-------------|
| Image upload latency (p95) | < 3s for 5MB image | APM |
| Audio transcription latency | < 1.5x real-time | APM |
| Document processing throughput | > 10 pages/sec | Benchmark |
| Cross-modal retrieval accuracy | > 80% relevance | Eval suite |
| Storage cost efficiency | < $0.02/GB/month | AWS billing |

---

## Open Questions

1. **Video support scope**: Full video or key frames + transcript only?
   - Recommendation: Key frames + transcript for v0.11.0; full video v0.12.0

2. **Local vision models**: Support LLaVA via Ollama?
   - Recommendation: Yes, for air-gapped deployments

3. **Image search UX**: Text-to-image vs. image-to-image vs. both?
   - Recommendation: Both; text-to-image primary, image-to-image optional

4. **Chunking strategy**: Fixed-size vs. structural for documents?
   - Recommendation: Structural by default, fixed-size as fallback

5. **Cost management**: Per-attachment billing or bundled?
   - Recommendation: Track usage, expose metrics; billing policy is deployment decision

---

## Appendix: Competitive Comparison

| Feature | Mnemo (Target) | Mem0 | Zep | Letta |
|---------|----------------|------|-----|-------|
| Image upload | ✅ | ✅ | ⚠️ | ❌ |
| Image description (vision) | ✅ | ✅ | ❌ | ❌ |
| Image embedding (CLIP) | ✅ | ❌ | ❌ | ❌ |
| Image-to-image search | ✅ | ❌ | ❌ | ❌ |
| Audio upload | ✅ | ❌ | ❌ | ❌ |
| Transcription | ✅ | ❌ | ❌ | ❌ |
| Speaker diarization | ✅ | ❌ | ❌ | ❌ |
| PDF parsing | ✅ | ✅ | ⚠️ | ⚠️ |
| Office docs | ✅ | ⚠️ | ❌ | ❌ |
| Structural chunking | ✅ | ❌ | ❌ | ❌ |
| Cross-modal entity linking | ✅ | ❌ | ❌ | ❌ |
| Multi-modal context retrieval | ✅ | ⚠️ | ❌ | ❌ |

Mnemo's multi-modal implementation will be the most comprehensive in the category, with unique features like CLIP embeddings for image-to-image search and cross-modal entity linking via the knowledge graph.
