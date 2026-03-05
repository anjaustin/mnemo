#!/usr/bin/env node
/**
 * Standalone falsification test for the Mnemo AnythingLLM provider.
 *
 * Prerequisites:
 *   - Mnemo server running at MNEMO_ENDPOINT (default http://localhost:8080)
 *   - Qdrant running (Mnemo's backend)
 *
 * Run:
 *   node integrations/anythingllm/test.js
 *
 * Tests the raw vector API surface that the AnythingLLM provider relies on,
 * plus the provider class itself in standalone mode.
 */

const { Mnemo } = require("./index");

const NAMESPACE = `__mnemo_test_${Date.now()}`;
const DIM = 384; // Common embedding dimension

let passed = 0;
let failed = 0;

function assert(condition, message) {
  if (!condition) {
    console.error(`  FAIL: ${message}`);
    failed++;
  } else {
    console.log(`  PASS: ${message}`);
    passed++;
  }
}

function randomVector(dim) {
  return Array.from({ length: dim }, () => Math.random() * 2 - 1);
}

async function run() {
  const mnemo = new Mnemo();
  console.log(`\nMnemo AnythingLLM Provider Test`);
  console.log(`Endpoint: ${mnemo.endpoint}`);
  console.log(`Namespace: ${NAMESPACE}\n`);

  // ─── 1. Health / connect ─────────────────────────────────────
  console.log("1. Connection & health");
  try {
    const conn = await mnemo.connect();
    assert(conn.client === mnemo, "connect() returns client reference");
    assert(typeof conn.version === "string", "connect() returns version string");
  } catch (e) {
    assert(false, `connect() threw: ${e.message}`);
  }

  try {
    const hb = await mnemo.heartbeat();
    assert(hb.heartbeat > 0, "heartbeat() returns positive timestamp");
  } catch (e) {
    assert(false, `heartbeat() threw: ${e.message}`);
  }

  // ─── 2. Namespace lifecycle ──────────────────────────────────
  console.log("\n2. Namespace lifecycle");

  assert(
    (await mnemo.hasNamespace(NAMESPACE)) === false,
    "namespace does not exist initially"
  );

  assert(
    (await mnemo.namespace(mnemo, NAMESPACE)) === null,
    "namespace() returns null for non-existent"
  );

  // ─── 3. Upsert vectors ──────────────────────────────────────
  console.log("\n3. Vector upsert");

  const vectors = [];
  for (let i = 0; i < 10; i++) {
    vectors.push({
      id: `vec-${i}`,
      vector: randomVector(DIM),
      metadata: {
        text: `This is test document ${i}`,
        docId: `doc-${i % 3}`, // 3 docs, multiple vectors each
        source: "test",
      },
    });
  }

  // Direct API call (simulating what addDocumentToNamespace does internally)
  const upsertResult = await mnemo._post(
    `/api/v1/vectors/${encodeURIComponent(NAMESPACE)}`,
    { vectors }
  );
  assert(upsertResult.ok === true, "upsert returns ok:true");
  assert(upsertResult.upserted === 10, "upsert reports 10 vectors");

  // ─── 4. Namespace now exists ─────────────────────────────────
  console.log("\n4. Namespace exists after upsert");

  assert(
    (await mnemo.hasNamespace(NAMESPACE)) === true,
    "namespace exists after upsert"
  );

  const nsInfo = await mnemo.namespace(mnemo, NAMESPACE);
  assert(nsInfo !== null, "namespace() returns non-null");
  assert(nsInfo.name === NAMESPACE, "namespace() returns correct name");

  // ─── 5. Count ────────────────────────────────────────────────
  console.log("\n5. Vector count");

  const count = await mnemo.namespaceCount(NAMESPACE);
  assert(count === 10, `count is 10 (got ${count})`);

  // ─── 6. Similarity search ───────────────────────────────────
  console.log("\n6. Similarity search");

  // Query with a vector close to vec-0
  const queryVector = vectors[0].vector.map((v) => v + (Math.random() * 0.01 - 0.005));

  const searchResult = await mnemo.similarityResponse({
    client: mnemo,
    namespace: NAMESPACE,
    queryVector,
    similarityThreshold: 0.0,
    topN: 5,
  });

  assert(
    searchResult.contextTexts.length > 0,
    `search returned ${searchResult.contextTexts.length} results`
  );
  assert(
    searchResult.sourceDocuments.length === searchResult.contextTexts.length,
    "sourceDocuments count matches contextTexts"
  );
  assert(
    searchResult.scores.length === searchResult.contextTexts.length,
    "scores count matches contextTexts"
  );
  assert(
    searchResult.scores[0] > 0.5,
    `top result score is reasonable (${searchResult.scores[0].toFixed(3)})`
  );

  // ─── 7. Delete specific vectors ─────────────────────────────
  console.log("\n7. Delete specific vectors");

  await mnemo._post(
    `/api/v1/vectors/${encodeURIComponent(NAMESPACE)}/delete`,
    { ids: ["vec-0", "vec-1", "vec-2"] }
  );

  const countAfterDelete = await mnemo.namespaceCount(NAMESPACE);
  assert(countAfterDelete === 7, `count after deleting 3 is 7 (got ${countAfterDelete})`);

  // ─── 8. Namespace stats (AnythingLLM UI handler) ─────────────
  console.log("\n8. Namespace stats");

  const stats = await mnemo["namespace-stats"]({ namespace: NAMESPACE });
  assert(stats.vectorCount === 7, `stats vectorCount is 7 (got ${stats.vectorCount})`);

  // ─── 9. Delete namespace ─────────────────────────────────────
  console.log("\n9. Delete namespace");

  const deleteResult = await mnemo["delete-namespace"]({ namespace: NAMESPACE });
  assert(
    deleteResult.message.includes(NAMESPACE),
    "delete-namespace returns message with namespace name"
  );

  assert(
    (await mnemo.hasNamespace(NAMESPACE)) === false,
    "namespace gone after delete"
  );

  // ─── 10. Reset (safety no-op) ────────────────────────────────
  console.log("\n10. Reset");

  const resetResult = await mnemo.reset();
  assert(resetResult.reset === true, "reset returns {reset: true}");

  // ─── 11. curateSources ──────────────────────────────────────
  console.log("\n11. curateSources");

  const curated = mnemo.curateSources([
    { text: "hello", vector: [1, 2, 3], score: 0.9, docId: "x" },
    { text: "world", score: 0.8 },
  ]);
  assert(curated[0].vector === undefined, "curateSources strips vector field");
  assert(curated[0].score === 0.9, "curateSources preserves score");
  assert(curated[0].text === "hello", "curateSources preserves text");

  // ─── 12. Batch upsert (500+ vectors) ────────────────────────
  console.log("\n12. Batch upsert (>500 vectors)");

  const batchNs = `${NAMESPACE}_batch`;
  const bigBatch = [];
  for (let i = 0; i < 600; i++) {
    bigBatch.push({
      id: `batch-${i}`,
      vector: randomVector(DIM),
      metadata: { text: `batch doc ${i}` },
    });
  }

  await mnemo._post(
    `/api/v1/vectors/${encodeURIComponent(batchNs)}`,
    { vectors: bigBatch }
  );

  const batchCount = await mnemo.namespaceCount(batchNs);
  assert(batchCount === 600, `batch count is 600 (got ${batchCount})`);

  // Cleanup
  await mnemo._delete(`/api/v1/vectors/${encodeURIComponent(batchNs)}`);

  // ─── Summary ────────────────────────────────────────────────
  console.log(`\n${"=".repeat(50)}`);
  console.log(`Results: ${passed} passed, ${failed} failed`);
  console.log(`${"=".repeat(50)}\n`);

  process.exit(failed > 0 ? 1 : 0);
}

run().catch((err) => {
  console.error("Test runner error:", err);
  process.exit(1);
});
