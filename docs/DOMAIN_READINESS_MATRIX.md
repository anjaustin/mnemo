# Domain Readiness Matrix

This document tracks Mnemo's near-term readiness for high-value application domains where memory quality, temporal correctness, and identity continuity matter.

## Readiness Snapshot

| Domain | Readiness | Why it fits now | Primary gaps | Next milestone |
|---|---|---|---|---|
| Scientific research assistance | High | Temporal memory, contradiction handling, history preservation | Better provenance and citation surfacing | Citation fields in context output |
| Specialized legal support | Medium-High | As-of retrieval and audit trails map directly to legal workflows | Policy controls (retention/redaction/access) | Domain policy layer |
| Legacy simulation / continuity agents | High | Identity substrate plus evolving experience memory | Canon-lock and world-state partitioning | Memory partition model |
| MMORPG character actors | Medium-High | Multi-agent continuity and temporal event memory | Scale and shard-aware routing | NPC load + shard tests |
| Mentor / teacher / companion agents | High | Long-horizon personalization and preference drift support | User-facing memory governance | End-user memory controls |
| Creative writing systems | High | Canon consistency and historical timeline support | Author-facing contradiction explanation | Canon debugger endpoint |
| ML/AI/AGI R&D assistance | Very High | Experiment timelines and evolving assumptions | Artifact linking and report generation | Experiment-memory schema |
| Support / customer success copilots | Very High | Session continuity and state-change recall | CRM schema templates | Vertical reference pack |
| SRE / incident copilots | High | Incident timeline and rollback-aware recall | Incident source adapters | Incident eval pack |

## Cross-Domain Platform Priorities

1. Provenance-by-default in context responses (source event IDs, timestamps, confidence).
2. Policy controls for retention, redaction, and access constraints.
3. Memory partition contracts (`identity`, `episodic`, `world`, `operational`).
4. Domain evaluation packs with reproducible case sets and score thresholds.
5. Retrieval explainability endpoints for answer traceability.

## 30/60/90 Plan

### 30 days

- Add provenance fields to retrieval responses.
- Add domain eval packs for research, legal, NPC, mentor, creative, and R&D scenarios.
- Add baseline policy configuration hooks.
- Add retrieval explain diagnostics endpoint.

### 60 days

- Ship two reference implementations (research assistant and support copilot).
- Add partitioned memory policy enforcement.
- Add multi-agent scale tests.
- Publish competitive benchmark v1 artifacts.

### 90 days

- Publish three domain case studies with reproducible evaluation data.
- Ship production deployment profiles by domain.
- Add lifecycle governance APIs for enterprise operators.
- Publish stability policy labels (GA/Beta/Experimental) for core features.

## Current Focus

The active first domain pack is scientific research assistance.
See `eval/scientific_research_cases.json`, `eval/scientific_research_cases_v2.json`, and `docs/EVALUATION.md`.
