# Core Concepts

Understanding Mnemo's data model and architecture.

---

## In This Section

| Concept | Description |
|---------|-------------|
| **[Overview](overview.md)** | The big picture - what Mnemo does and how |
| **[Episodes](episodes.md)** | The atomic unit of memory |
| **[Entities & Edges](entities-and-edges.md)** | The knowledge graph |
| **[Temporal Model](temporal-model.md)** | How facts change over time |
| **[Evidence-First Memory](evidence-first-memory.md)** | Why evidence visibility matters in hybrid retrieval |

Sessions and Users are covered in the [Overview](overview.md).

---

## Key Ideas

### 1. Episodes are Input

Everything you tell Mnemo is an **episode** - a message, event, or document. Episodes are processed to extract knowledge.

### 2. Knowledge Graph is Output

From episodes, Mnemo extracts **entities** (people, products, concepts) and **edges** (facts connecting entities).

### 3. Time is First-Class

Every fact has `valid_at` (when it became true) and `invalid_at` (when superseded). Old facts aren't deleted - they're history.

### 4. Retrieval is Hybrid

Context assembly combines semantic search, full-text search, and graph traversal for comprehensive recall.

### 5. Evidence Matters

Hybrid retrieval is powerful, but different retrieval paths can disagree. Mnemo treats that as an important design consideration for future diagnostics, evaluation, and context assembly.

---

## Data Flow

```
Input: Episode ("Alice joined Acme Corp as VP Sales")
         │
         ▼
Processing: Extract entities (Alice, Acme Corp)
            Extract edges (Alice → works_at → Acme Corp)
            Generate embeddings
         │
         ▼
Storage: Redis (graph) + Qdrant (vectors)
         │
         ▼
Output: Context ("Alice works at Acme Corp as VP Sales")
```

---

## Start Here

1. **[Overview](overview.md)** - Start with the big picture
2. **[Episodes](episodes.md)** - Understand input types
3. **[Entities & Edges](entities-and-edges.md)** - Understand the graph
4. **[Temporal Model](temporal-model.md)** - Understand time handling
5. **[Evidence-First Memory](evidence-first-memory.md)** - Explore evidence visibility in hybrid retrieval
