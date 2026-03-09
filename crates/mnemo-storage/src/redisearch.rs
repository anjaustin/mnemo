//! RediSearch full-text search implementation.
//!
//! Uses RediSearch (included in Redis Stack) for BM25 full-text search
//! across entities, edges, and episodes.
//!
//! Index schema:
//! - `{prefix}idx:entities` — ON JSON, fields: name (TEXT), summary (TEXT), user_id (TAG)
//! - `{prefix}idx:edges`    — ON JSON, fields: fact (TEXT), label (TAG), user_id (TAG)
//! - `{prefix}idx:episodes` — ON JSON, fields: content (TEXT), user_id (TAG), session_id (TAG)

use uuid::Uuid;

use mnemo_core::error::MnemoError;
use mnemo_core::traits::fulltext::FullTextStore;
use mnemo_core::traits::storage::StorageResult;

use crate::RedisStateStore;

impl FullTextStore for RedisStateStore {
    async fn search_entities_ft(
        &self,
        user_id: Uuid,
        query: &str,
        limit: u32,
    ) -> StorageResult<Vec<(Uuid, f32)>> {
        self.ft_search(
            &self.ft_index_name("entities"),
            query,
            &user_id.to_string(),
            limit,
        )
        .await
    }

    async fn search_edges_ft(
        &self,
        user_id: Uuid,
        query: &str,
        limit: u32,
    ) -> StorageResult<Vec<(Uuid, f32)>> {
        self.ft_search(
            &self.ft_index_name("edges"),
            query,
            &user_id.to_string(),
            limit,
        )
        .await
    }

    async fn search_episodes_ft(
        &self,
        user_id: Uuid,
        query: &str,
        limit: u32,
    ) -> StorageResult<Vec<(Uuid, f32)>> {
        self.ft_search(
            &self.ft_index_name("episodes"),
            query,
            &user_id.to_string(),
            limit,
        )
        .await
    }

    async fn ensure_indexes(&self) -> StorageResult<()> {
        // Entity index
        self.create_ft_index(
            &self.ft_index_name("entities"),
            &self.key(&["entity:"]),
            &[
                ("$.name", "name", "TEXT", Some("2.0")),
                ("$.summary", "summary", "TEXT", None),
                ("$.user_id", "user_id", "TAG", None),
                ("$.entity_type", "entity_type", "TAG", None),
            ],
        )
        .await?;

        // Edge index
        self.create_ft_index(
            &self.ft_index_name("edges"),
            &self.key(&["edge:"]),
            &[
                ("$.fact", "fact", "TEXT", None),
                ("$.label", "label", "TAG", None),
                ("$.user_id", "user_id", "TAG", None),
            ],
        )
        .await?;

        // Episode index
        self.create_ft_index(
            &self.ft_index_name("episodes"),
            &self.key(&["episode:"]),
            &[
                ("$.content", "content", "TEXT", None),
                ("$.user_id", "user_id", "TAG", None),
                ("$.session_id", "session_id", "TAG", None),
            ],
        )
        .await?;

        tracing::info!("RediSearch indexes ensured");
        Ok(())
    }
}

// ─── RediSearch helpers on RedisStateStore ──────────────────────────

impl RedisStateStore {
    fn ft_index_name(&self, resource: &str) -> String {
        format!("{}idx:{}", self.prefix, resource)
    }

    /// Create a RediSearch JSON index if it doesn't exist.
    async fn create_ft_index(
        &self,
        index_name: &str,
        prefix: &str,
        fields: &[(&str, &str, &str, Option<&str>)],
    ) -> StorageResult<()> {
        let mut conn = self.conn.clone();

        // Check if index exists (FT.INFO returns error if not)
        let exists: Result<redis::Value, _> = redis::cmd("FT.INFO")
            .arg(index_name)
            .query_async(&mut conn)
            .await;

        if exists.is_ok() {
            return Ok(()); // Already exists
        }

        // Build FT.CREATE command
        let mut cmd = redis::cmd("FT.CREATE");
        cmd.arg(index_name)
            .arg("ON")
            .arg("JSON")
            .arg("PREFIX")
            .arg("1")
            .arg(prefix)
            .arg("SCHEMA");

        for (json_path, alias, field_type, weight) in fields {
            cmd.arg(*json_path).arg("AS").arg(*alias).arg(*field_type);
            if let Some(w) = weight {
                cmd.arg("WEIGHT").arg(*w);
            }
        }

        cmd.exec_async(&mut conn).await.map_err(|e| {
            MnemoError::Redis(format!("FT.CREATE failed for {}: {}", index_name, e))
        })?;

        tracing::debug!(index = index_name, "Created RediSearch index");
        Ok(())
    }

    /// Execute an FT.SEARCH query filtered by user_id.
    async fn ft_search(
        &self,
        index_name: &str,
        query: &str,
        user_id: &str,
        limit: u32,
    ) -> StorageResult<Vec<(Uuid, f32)>> {
        let mut conn = self.conn.clone();

        // Escape special RediSearch characters in query text
        let escaped_query = escape_redisearch_query(query);

        // Escape hyphens in user_id for RediSearch TAG filter.
        // UUIDs contain hyphens which are special chars in TAG queries.
        let escaped_user_id = user_id.replace('-', "\\-");

        // Build query: user filter + text search
        let search_query = format!("@user_id:{{{}}} {}", escaped_user_id, escaped_query);

        let result: redis::Value = redis::cmd("FT.SEARCH")
            .arg(index_name)
            .arg(&search_query)
            .arg("LIMIT")
            .arg("0")
            .arg(limit.to_string())
            .arg("WITHSCORES")
            .arg("RETURN")
            .arg("0") // Don't return fields, just keys + scores
            .query_async(&mut conn)
            .await
            .map_err(|e| MnemoError::Redis(format!("FT.SEARCH failed: {}", e)))?;

        parse_ft_search_results(&self.prefix, result)
    }
}

/// Common English stop words that RediSearch would match on every document.
/// Filtering these out dramatically improves FT precision for natural-language
/// queries like "Where does Alice work?" → "Alice work".
const STOP_WORDS: &[&str] = &[
    "a", "an", "the", "and", "or", "but", "in", "on", "at", "to", "for", "of", "with", "by",
    "from", "is", "are", "was", "were", "be", "been", "being", "have", "has", "had", "do", "does",
    "did", "will", "would", "could", "should", "may", "might", "shall", "can", "need", "dare",
    "ought", "used", "it", "its", "this", "that", "these", "those", "i", "me", "my", "we", "our",
    "you", "your", "he", "him", "his", "she", "her", "they", "them", "their", "what", "which",
    "who", "whom", "where", "when", "why", "how", "all", "each", "every", "both", "few", "more",
    "most", "other", "some", "such", "no", "not", "only", "same", "so", "than", "too", "very",
    "just", "any", "about", "up", "out", "if", "because", "as", "while", "although", "since",
    "though", "into", "through", "during", "before", "after", "above", "below", "between", "then",
    "once", "here", "there", "s", "t", "don", "didn", "doesn", "isn", "wasn", "aren", "weren",
    "won", "hasn", "hadn", "does", "did", "do",
];

/// Strip stop words from a natural-language query and return meaningful keywords.
/// Falls back to the original query if all words are stop words.
fn extract_keywords(query: &str) -> String {
    let words: Vec<&str> = query
        .split_whitespace()
        .filter(|w| {
            let lower = w.to_lowercase();
            let clean: String = lower.chars().filter(|c| c.is_alphabetic()).collect();
            !clean.is_empty() && !STOP_WORDS.contains(&clean.as_str())
        })
        .collect();

    if words.is_empty() {
        // All words were stop words — use the original (RediSearch will handle it)
        query.trim().to_string()
    } else {
        words.join(" ")
    }
}

/// Escape special characters for RediSearch queries.
fn escape_redisearch_query(query: &str) -> String {
    // First extract meaningful keywords from natural-language queries
    let keywords = extract_keywords(query);

    // RediSearch special chars that need escaping in individual tokens
    let special = [
        '@', '!', '{', '}', '(', ')', '-', '=', '>', '[', ']', ':', ';', '~',
    ];

    // Join keywords with | (OR) so any single keyword hit returns the document.
    // AND (default) is too strict for natural-language queries where synonyms
    // differ between query and stored text ("prefer" vs "favourite").
    let escaped_tokens: Vec<String> = keywords
        .split_whitespace()
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut escaped = String::with_capacity(w.len() * 2);
            for ch in w.chars() {
                if special.contains(&ch) {
                    escaped.push('\\');
                }
                escaped.push(ch);
            }
            escaped
        })
        .collect();

    escaped_tokens.join("|")
}

/// Parse FT.SEARCH results with WITHSCORES into (Uuid, score) pairs.
///
/// FT.SEARCH with WITHSCORES returns:
/// [total_count, key1, score1, key2, score2, ...]
fn parse_ft_search_results(_prefix: &str, result: redis::Value) -> StorageResult<Vec<(Uuid, f32)>> {
    let items = match result {
        redis::Value::Array(items) => items,
        _ => return Ok(Vec::new()),
    };

    if items.is_empty() {
        return Ok(Vec::new());
    }

    // First element is total count, then alternating key/score pairs
    let mut results = Vec::new();
    let mut i = 1; // Skip total count

    while i + 1 < items.len() {
        // Key is like "mnemo:entity:019..."
        let key_str = match &items[i] {
            redis::Value::BulkString(bytes) => String::from_utf8_lossy(bytes).to_string(),
            redis::Value::SimpleString(s) => s.clone(),
            _ => {
                i += 2;
                continue;
            }
        };

        // Score
        let score: f32 = match &items[i + 1] {
            redis::Value::BulkString(bytes) => {
                String::from_utf8_lossy(bytes).parse().unwrap_or(0.0)
            }
            redis::Value::SimpleString(s) => s.parse().unwrap_or(0.0),
            _ => 0.0,
        };

        // Extract UUID from key: "prefix:resource:UUID"
        if let Some(uuid_str) = key_str.rsplit(':').next() {
            if let Ok(id) = Uuid::parse_str(uuid_str) {
                // Normalize score to 0.0-1.0 range (RediSearch scores vary)
                let normalized = (score / (score + 1.0)).min(1.0);
                results.push((id, normalized));
            }
        }

        i += 2;
    }

    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_escape_redisearch_query() {
        assert_eq!(escape_redisearch_query("hello world"), "hello|world");
        assert_eq!(
            escape_redisearch_query("user@email.com"),
            "user\\@email.com"
        );
        assert_eq!(
            escape_redisearch_query("INV-2024-0847"),
            "INV\\-2024\\-0847"
        );
        assert_eq!(escape_redisearch_query("tag:{value}"), "tag\\:\\{value\\}");
    }

    #[test]
    fn test_parse_empty_results() {
        let result = redis::Value::Array(vec![
            redis::Value::Int(0), // total count = 0
        ]);
        let parsed = parse_ft_search_results("mnemo:", result).unwrap();
        assert!(parsed.is_empty());
    }
}
