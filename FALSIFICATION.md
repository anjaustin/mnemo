# Mnemo Core — Falsification Report

**Reviewed:** All files in `crates/mnemo-core/`, `Cargo.toml`, `Dockerfile`, `docker-compose.yml`, `config/default.toml`
**Date:** 2026-03-01

---

## 🔴 CRITICAL — Will break compilation or cause runtime bugs

### 1. `edition = "2024"` does not exist
**File:** `Cargo.toml:14`
**Issue:** Rust edition `"2024"` was never stabilized under that name. The latest stable edition is `"2021"`. Rust 1.85 ships edition `"2024"` as experimental at best, but crates.io tooling and most CI won't accept it.
**Fix:** Change to `edition = "2021"`.

### 2. `thiserror = "2"` — major version mismatch
**File:** `Cargo.toml:29`
**Issue:** `thiserror` v2 exists but made breaking changes to the `#[error(...)]` attribute syntax. Need to verify our `error.rs` is compatible. If `thiserror` 2.x isn't widely adopted yet in the ecosystem, we may get dependency resolution conflicts with other crates that pin to 1.x.
**Fix:** Pin to `thiserror = "1"` for maximum ecosystem compatibility, or audit all downstream deps.

### 3. Unused imports in `context.rs`
**File:** `context.rs:5-7`
```rust
use super::edge::Edge;
use super::entity::Entity;
use super::episode::Episode;
```
**Issue:** `Edge`, `Entity`, and `Episode` are imported but never used anywhere in the file. This will cause `unused import` warnings and may fail CI with `#[deny(unused_imports)]`.
**Fix:** Remove the three unused imports.

### 4. `EntityType` serde roundtrip broken for `Custom` variant
**File:** `entity.rs:10-20`
**Issue:** `EntityType` uses `#[serde(rename_all = "snake_case")]` but the `Custom(String)` variant serializes as `{"custom": "medication"}` — a tagged enum. However, `from_str_flexible("medication")` returns `Custom("medication")`. If an LLM returns `"medication"` as a string and we try to deserialize it directly as `EntityType`, it will fail because serde expects the tagged format, not a bare string. The serde representation and the `from_str_flexible` function have incompatible contracts.
**Fix:** Either use `#[serde(untagged)]` with a custom deserializer, or make `from_str_flexible` the canonical deserialization path and use `#[serde(deserialize_with = "...")]`.

### 5. `CreateEpisodeRequest` serde rename will collide
**File:** `episode.rs:105`
```rust
#[serde(rename = "type")]
pub episode_type: EpisodeType,
```
**Issue:** `type` is a Rust keyword. While this serde rename works for JSON deserialization, the internal field name `episode_type` is used in `Episode` struct (line 44) without the rename. So `Episode` serializes as `"episode_type": "message"` while `CreateEpisodeRequest` serializes as `"type": "message"`. This API inconsistency will confuse SDK developers.
**Fix:** Add the same `#[serde(rename = "type")]` to `Episode.episode_type`, or use `"episode_type"` consistently everywhere (including the request).

### 6. `EdgeFilter::default()` has `limit: 0`
**File:** `edge.rs:89`
**Issue:** `EdgeFilter` derives `Default`, which gives `limit` the value `0` (u32 default). But `default_limit()` returns `100`. The `#[serde(default = "default_limit")]` only applies during JSON deserialization, not when using `Default::default()` in Rust code. All tests using `EdgeFilter { ..Default::default() }` have `limit: 0`, which would return zero results in a real storage implementation.
**Fix:** Implement `Default` manually for `EdgeFilter` instead of deriving it, setting `limit: 100`.

---

## 🟡 DESIGN ISSUES — Won't crash but will cause pain at scale

### 7. `episode_ids: Vec<Uuid>` on Entity will grow unbounded
**File:** `entity.rs:69`
**Issue:** An entity mentioned in 100,000 episodes will have a 100,000-element Vec serialized into Redis on every update. This is O(n) on every mention recording (linear scan for dedup in `record_mention`) and O(n) storage. At scale, a popular entity like "the user themselves" will be mentioned in nearly every episode.
**Fix:** Remove `episode_ids` from `Entity`. Track the entity→episode relationship in a separate Redis sorted set (`entity_episodes:{entity_id}`) or rely on Qdrant's payload filtering. Keep `mention_count` as an atomic counter.

### 8. `Entity::record_mention` linear scan
**File:** `entity.rs:120-126`
**Issue:** `self.episode_ids.contains(&episode_id)` is O(n). For a high-frequency entity, this becomes a hot path bottleneck.
**Fix:** Addressed by fix #7 (remove the Vec). If kept, use a `HashSet<Uuid>` instead.

### 9. No validation on `User.name` or `User.external_id`
**File:** `user.rs`
**Issue:** Empty string `""` is accepted for `name`. Extremely long strings (megabytes) are accepted for all fields. `external_id` could contain control characters, newlines, etc. No max length on `metadata` JSON blob.
**Fix:** Add a `validate()` method or use a validation crate. At minimum: non-empty name, reasonable max lengths (e.g., 256 chars for name, 512 for external_id, 1MB for metadata).

### 10. No validation on `Episode.content`
**File:** `episode.rs`
**Issue:** An episode with 10MB of content will be stored, sent to the LLM for extraction, and embedded — all without any size check. This is a DoS vector and an API cost bomb.
**Fix:** Add `max_content_length` to config. Validate on ingestion. Default to something sane like 100KB.

### 11. `UpdateUserRequest` can't clear optional fields
**File:** `user.rs:85-100`
**Issue:** There's no way to set `email` back to `None` or `external_id` back to `None`. `None` in the update means "don't change," but there's no sentinel for "clear this field." This is the classic PATCH problem.
**Fix:** Use a three-state wrapper: `enum Patch<T> { Unchanged, Clear, Set(T) }` or accept `serde_json::Value::Null` as the "clear" signal.

### 12. `ContextBlock::assemble` doesn't account for section headers in token budget
**File:** `context.rs:233-293`
**Issue:** The section headers ("Known entities:\n", "Current facts:\n", "Relevant conversation history:\n") are not counted toward the token budget. Each header is ~5-8 tokens. With a tight budget (e.g., 50 tokens), the headers alone eat 15-24 tokens — nearly half the budget — untracked.
**Fix:** Count header tokens before adding items.

### 13. `ContextBlock::assemble` adds empty sections
**File:** `context.rs:238-253`
**Issue:** If `entities` is non-empty but the first entity exceeds the remaining budget, the section header "Known entities:\n" is still pushed to `parts` with no items under it. Wastes tokens and looks bad.
**Fix:** Only push the section to `parts` if at least one item was added.

### 14. Storage traits import types they don't use
**File:** `storage.rs:4-5`
**Issue:** `ContextBlock`, `ContextRequest`, `SearchRequest`, `SearchResult`, `ExtractedRelationship`, `ExtractedEntity`, `ProcessingStatus` are all imported but never appear in any trait method signature. Unused imports.
**Fix:** Remove unused imports. Context/search operations belong in a separate `RetrievalEngine` trait, not in raw storage.

### 15. `MnemoStore` composite trait is too monolithic
**File:** `storage.rs:211-220`
**Issue:** Forcing a single struct to implement `UserStore + SessionStore + EpisodeStore + EntityStore + EdgeStore + VectorStore` means Redis and Qdrant must be behind the same struct. But Redis handles state and Qdrant handles vectors — they're fundamentally different backends. This forces an awkward wrapper struct.
**Fix:** Split into `StateStore` (Redis: users, sessions, episodes, entities, edges) and `VectorStore` (Qdrant). The server layer composes them, not a single trait.

### 16. `invalidated_by_episode_id` name is misleading
**File:** `edge.rs:53`
**Issue:** The field is named `invalidated_by_episode_id` but it stores the episode that *caused* the invalidation, not the episode that "invalidated by" (which reads as passive). The `invalidate()` method parameter is named `invalidated_by`, which makes it sound like it's the edge that's doing the invalidating.
**Fix:** Rename to `invalidating_episode_id` or `invalidation_source_episode_id`.

---

## 🟢 MINOR — Cleanup and hardening

### 17. `estimate_tokens` is byte-based, not char-based
**File:** `context.rs:212-214`
**Issue:** `text.len()` returns bytes, not characters. For UTF-8 text with non-ASCII characters (common in multilingual agent applications), this overcounts. "こんにちは" is 15 bytes but ~5 tokens.
**Fix:** Use `text.chars().count()` or, better, a proper tokenizer. Document the limitation for now.

### 18. `chrono::Duration::days()` deprecation
**File:** `edge.rs:255-256`
**Issue:** `chrono::Duration::days(30)` — in recent chrono versions, some `Duration` constructors have been deprecated in favor of `TimeDelta`. May produce deprecation warnings.
**Fix:** Use `chrono::TimeDelta::days(30)` or `chrono::Duration::try_days(30).unwrap()`.

### 19. No `PartialEq` on most domain types
**File:** All model files
**Issue:** `User`, `Session`, `Episode`, `Entity`, `Edge` don't derive `PartialEq`. This makes testing harder — you can't `assert_eq!(user_a, user_b)` directly.
**Fix:** Add `PartialEq` derive to all domain types.

### 20. Docker Compose uses `version` key (deprecated)
**File:** `docker-compose.yml:1`
**Issue:** `version: "3.9"` is deprecated in modern Docker Compose. It's ignored by Compose V2 and produces a warning.
**Fix:** Remove the `version` line entirely.

### 21. Dockerfile references `mnemo-server` binary that doesn't exist
**File:** `Dockerfile:40`
**Issue:** `cargo build --release --bin mnemo-server` — but there's no `[[bin]]` defined anywhere. The `mnemo-server` crate is a library (`lib.rs`), not a binary. The build will fail.
**Fix:** Either add a `src/main.rs` to `mnemo-server` or add `[[bin]]` in its `Cargo.toml`, or change the Dockerfile to reference the correct binary.

### 22. No `Cargo.lock` file
**Issue:** The workspace doesn't have a `Cargo.lock`. For an application (not a library), the lock file should be committed to ensure reproducible builds.
**Fix:** Generate and commit `Cargo.lock`.

### 23. Config TOML uses `[embedding]` but code uses `EmbeddingConfig`
**File:** `config/default.toml` vs `traits/llm.rs`
**Issue:** No config deserialization code exists yet, but the TOML section name `[embedding]` doesn't match the struct name pattern. Minor, but worth noting for consistency when we build the config parser.

---

## Summary

| Severity | Count | Action |
|----------|-------|--------|
| 🔴 Critical | 6 | Must fix before any compilation attempt |
| 🟡 Design | 10 | Fix before building storage layer |
| 🟢 Minor | 7 | Fix during cleanup pass |

**Recommendation:** Fix all 🔴 critical issues now. Fix 🟡 design issues #7, #9, #10, #15 before starting `mnemo-storage` — they directly affect the storage trait contracts. The rest can be addressed incrementally.
