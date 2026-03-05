/**
 * Mnemo Vector Database Provider for AnythingLLM
 *
 * Drop this directory into:
 *   anything-llm/server/utils/vectorDbProviders/mnemo/
 *
 * Then register it in:
 *   anything-llm/server/utils/helpers/index.js  (add case to getVectorDbClass)
 *   anything-llm/frontend/src/pages/GeneralSettings/VectorDatabase/index.jsx
 *
 * Environment variables:
 *   MNEMO_ENDPOINT  - Mnemo server URL (default: http://localhost:8080)
 *   MNEMO_API_KEY   - Optional API key for authentication
 *
 * @see https://github.com/your-org/mnemo for Mnemo documentation
 */

const { v4: uuidv4 } = require("uuid");

// AnythingLLM base class & helpers — resolved relative to AnythingLLM's tree.
// When running standalone tests, these are stubbed.
let VectorDatabase, toChunks, getEmbeddingEngineSelection, TextSplitter;
let storeVectorResult, cachedVectorInformation, sourceIdentifier;
let DocumentVectors;

try {
  ({ VectorDatabase } = require("../base"));
  ({ toChunks, getEmbeddingEngineSelection } = require("../../helpers"));
  TextSplitter = require("../../TextSplitter");
  ({ storeVectorResult, cachedVectorInformation } = require("../../files"));
  ({ sourceIdentifier } = require("../../chats"));
  DocumentVectors = require("../../../models/vectors");
} catch (_) {
  // Running outside AnythingLLM (tests, standalone). Provide minimal stubs.
  VectorDatabase = class {
    logger(msg, ...args) {
      console.log(`[Mnemo] ${msg}`, ...args);
    }
  };
  toChunks = (arr, size) =>
    Array.from({ length: Math.ceil(arr.length / size) }, (_, i) =>
      arr.slice(i * size, i * size + size)
    );
  getEmbeddingEngineSelection = () => null;
  TextSplitter = null;
  storeVectorResult = async () => {};
  cachedVectorInformation = async () => ({ exists: false });
  sourceIdentifier = (doc) =>
    doc?.title && doc?.published
      ? `title:${doc.title}-timestamp:${doc.published}`
      : uuidv4();
  DocumentVectors = {
    bulkInsert: async () => {},
    where: async () => [],
    deleteIds: async () => {},
  };
}

class Mnemo extends VectorDatabase {
  constructor() {
    super();
    this.endpoint =
      process.env.MNEMO_ENDPOINT || "http://localhost:8080";
    this.apiKey = process.env.MNEMO_API_KEY || null;
  }

  get name() {
    return "Mnemo";
  }

  // ─── HTTP helpers ──────────────────────────────────────────────

  /** Build headers for Mnemo API requests. */
  _headers() {
    const h = { "Content-Type": "application/json" };
    if (this.apiKey) {
      h["Authorization"] = `Bearer ${this.apiKey}`;
    }
    return h;
  }

  /** Low-level fetch wrapper with error handling. */
  async _fetch(path, options = {}) {
    const url = `${this.endpoint}${path}`;
    const res = await fetch(url, {
      ...options,
      headers: { ...this._headers(), ...(options.headers || {}) },
    });
    if (!res.ok) {
      const text = await res.text().catch(() => "");
      throw new Error(
        `Mnemo API error ${res.status} on ${options.method || "GET"} ${path}: ${text}`
      );
    }
    return res.json();
  }

  async _get(path) {
    return this._fetch(path, { method: "GET" });
  }

  async _post(path, body) {
    return this._fetch(path, {
      method: "POST",
      body: JSON.stringify(body),
    });
  }

  async _delete(path) {
    return this._fetch(path, { method: "DELETE" });
  }

  // ─── Connection & health ───────────────────────────────────────

  async connect() {
    const health = await this._get("/health");
    if (health.status !== "ok") {
      throw new Error("Mnemo health check failed");
    }
    return { client: this, version: health.version };
  }

  async heartbeat() {
    const health = await this._get("/health");
    return { heartbeat: health.status === "ok" ? Date.now() : 0 };
  }

  // ─── Namespace operations ─────────────────────────────────────

  async totalVectors() {
    // AnythingLLM calls this on the settings page. Since we can't enumerate
    // all namespaces cheaply, return 0 and let per-namespace counts work.
    return 0;
  }

  async namespaceCount(namespace = "") {
    const data = await this._get(
      `/api/v1/vectors/${encodeURIComponent(namespace)}/count`
    );
    return data.count || 0;
  }

  async namespace(client, namespace = "") {
    const data = await this._get(
      `/api/v1/vectors/${encodeURIComponent(namespace)}/exists`
    );
    return data.exists ? { name: namespace } : null;
  }

  async hasNamespace(namespace = "") {
    const data = await this._get(
      `/api/v1/vectors/${encodeURIComponent(namespace)}/exists`
    );
    return data.exists;
  }

  async namespaceExists(client, namespace = "") {
    return this.hasNamespace(namespace);
  }

  async deleteVectorsInNamespace(client, namespace = "") {
    await this._delete(`/api/v1/vectors/${encodeURIComponent(namespace)}`);
    return true;
  }

  // ─── Document ingestion ───────────────────────────────────────

  async addDocumentToNamespace(
    namespace,
    documentData = {},
    fullFilePath = null,
    skipCache = false
  ) {
    const { pageContent, docId, ...metadata } = documentData;

    if (!pageContent || pageContent.length === 0) {
      return { vectorized: false, error: "No page content to vectorize." };
    }

    // Check embedding cache
    if (!skipCache) {
      const cached = await cachedVectorInformation(fullFilePath);
      if (cached.exists) {
        const { chunks } = cached;
        const vectors = chunks.map((chunk) => ({
          id: chunk.vectorDbId || uuidv4(),
          vector: chunk.values,
          metadata: chunk.metadata || {},
        }));

        if (vectors.length > 0) {
          // Upsert cached vectors in batches
          for (const batch of toChunks(vectors, 500)) {
            await this._post(
              `/api/v1/vectors/${encodeURIComponent(namespace)}`,
              { vectors: batch }
            );
          }

          // Track in DocumentVectors
          await DocumentVectors.bulkInsert(
            vectors.map((v) => ({ docId, vectorId: v.id }))
          );
        }

        return { vectorized: true, error: null };
      }
    }

    // Split text into chunks
    const EmbedderEngine = getEmbeddingEngineSelection();
    if (!EmbedderEngine || !TextSplitter) {
      return {
        vectorized: false,
        error: "Embedding engine or TextSplitter not available.",
      };
    }

    const textSplitter = new TextSplitter({
      chunkSize: TextSplitter.determineMaxChunkSize(
        null,
        EmbedderEngine.embeddingMaxChunkLength
      ),
      chunkOverlap: 20,
      chunkHeaderMeta: TextSplitter.buildHeaderMeta
        ? TextSplitter.buildHeaderMeta(metadata)
        : null,
    });

    const textChunks = await textSplitter.splitText(pageContent);
    if (textChunks.length === 0) {
      return { vectorized: false, error: "No text chunks produced." };
    }

    // Embed chunks
    const embeddingChunks = toChunks(textChunks, EmbedderEngine.embeddingMaxChunkLength || 1000);
    const allVectors = [];

    for (const chunk of embeddingChunks) {
      const embeddings = await EmbedderEngine.embedChunks(chunk);
      if (!embeddings || embeddings.length === 0) continue;

      for (let i = 0; i < chunk.length; i++) {
        if (!embeddings[i]) continue;
        const vectorId = uuidv4();
        allVectors.push({
          id: vectorId,
          vector: embeddings[i],
          metadata: {
            ...metadata,
            text: chunk[i],
            docId,
          },
        });
      }
    }

    if (allVectors.length === 0) {
      return { vectorized: false, error: "Embedding produced no vectors." };
    }

    // Upsert to Mnemo in batches of 500
    for (const batch of toChunks(allVectors, 500)) {
      await this._post(
        `/api/v1/vectors/${encodeURIComponent(namespace)}`,
        { vectors: batch }
      );
    }

    // Track in DocumentVectors
    await DocumentVectors.bulkInsert(
      allVectors.map((v) => ({ docId, vectorId: v.id }))
    );

    // Cache for future use
    await storeVectorResult(
      allVectors.map((v) => ({
        vectorDbId: v.id,
        values: v.vector,
        metadata: v.metadata,
      })),
      fullFilePath
    );

    return { vectorized: true, error: null };
  }

  // ─── Document deletion ────────────────────────────────────────

  async deleteDocumentFromNamespace(namespace, docId) {
    const knownVectors = await DocumentVectors.where({ docId });
    if (knownVectors.length === 0) return true;

    const vectorIds = knownVectors.map((v) => v.vectorId);

    // Delete from Mnemo in batches
    for (const batch of toChunks(vectorIds, 500)) {
      await this._post(
        `/api/v1/vectors/${encodeURIComponent(namespace)}/delete`,
        { ids: batch }
      );
    }

    // Remove tracking records
    const dbIds = knownVectors.map((v) => v.id);
    await DocumentVectors.deleteIds(dbIds);

    return true;
  }

  // ─── Similarity search ────────────────────────────────────────

  async performSimilaritySearch({
    namespace = "",
    input = "",
    LLMConnector = null,
    similarityThreshold = 0.25,
    topN = 4,
    filterIdentifiers = [],
  }) {
    const EmbedderEngine = getEmbeddingEngineSelection();
    if (!EmbedderEngine) {
      return {
        contextTexts: [],
        sources: [],
        message:
          "No embedding engine available for similarity search.",
      };
    }

    // Embed the query
    const queryEmbeddings = await EmbedderEngine.embedChunks([input]);
    if (!queryEmbeddings || queryEmbeddings.length === 0) {
      return {
        contextTexts: [],
        sources: [],
        message: "Failed to embed query.",
      };
    }
    const queryVector = queryEmbeddings[0];

    const { contextTexts, sourceDocuments, scores } =
      await this.similarityResponse({
        client: this,
        namespace,
        queryVector,
        similarityThreshold,
        topN,
        filterIdentifiers,
      });

    const sources = sourceDocuments.map((doc, i) => ({
      ...doc,
      score: scores[i] || 0,
    }));

    return {
      contextTexts,
      sources: this.curateSources(sources),
      message: false,
    };
  }

  async similarityResponse({
    client,
    namespace,
    queryVector,
    similarityThreshold = 0.25,
    topN = 4,
    filterIdentifiers = [],
  }) {
    const data = await this._post(
      `/api/v1/vectors/${encodeURIComponent(namespace)}/query`,
      {
        vector: queryVector,
        top_k: topN,
        min_score: similarityThreshold,
      }
    );

    const results = data.results || [];

    // Filter out pinned/known sources if filterIdentifiers provided
    const filtered =
      filterIdentifiers.length > 0
        ? results.filter((r) => {
            const srcId = sourceIdentifier(r.payload || {});
            return !filterIdentifiers.includes(srcId);
          })
        : results;

    const contextTexts = [];
    const sourceDocuments = [];
    const scores = [];

    for (const hit of filtered) {
      const payload = hit.payload || {};
      const text = payload.text || "";
      contextTexts.push(text);
      sourceDocuments.push(payload);
      scores.push(hit.score || 0);
    }

    return { contextTexts, sourceDocuments, scores };
  }

  // ─── Namespace management (AnythingLLM UI handlers) ───────────

  async "namespace-stats"(reqBody = {}) {
    const { namespace = null } = reqBody;
    if (!namespace) throw new Error("namespace required");

    const exists = await this.hasNamespace(namespace);
    if (!exists) return {};

    const count = await this.namespaceCount(namespace);
    return {
      namespace,
      vectorCount: count,
    };
  }

  async "delete-namespace"(reqBody = {}) {
    const { namespace = null } = reqBody;
    if (!namespace) throw new Error("namespace required");

    await this.deleteVectorsInNamespace(this, namespace);
    return {
      message: `Namespace ${namespace} deleted from Mnemo.`,
    };
  }

  async reset() {
    // AnythingLLM "reset" clears everything. For safety, this is a no-op
    // that returns success. Operators should delete namespaces individually.
    this.logger(
      "Reset called — no-op for safety. Delete namespaces individually."
    );
    return { reset: true };
  }

  // ─── Source curation ──────────────────────────────────────────

  curateSources(sources = []) {
    return sources.map((source) => {
      const { vector, score, ...rest } = source;
      return { ...rest, ...(score !== undefined ? { score } : {}) };
    });
  }
}

module.exports = { Mnemo };
