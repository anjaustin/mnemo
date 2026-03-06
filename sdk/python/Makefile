# ============================================================================
# Mnemo Python SDK — Developer Makefile
#
# Targets:
#   make test        — build server image, start test stack, run full suite,
#                      tear down.  Single-command reproducible test run.
#   make test-local  — run tests against an already-running server on :8080
#                      (no Docker needed; useful for fast iteration).
#   make lint        — run ruff + mypy (if installed).
#   make clean       — remove .pyc files and __pycache__ dirs.
#   make down        — tear down the test Docker Compose stack.
#
# Prerequisites:
#   - Docker + Docker Compose v2 (for `make test`)
#   - Python 3.10+ with pytest, pytest-asyncio, aioresponses installed
#     (pip install -e ".[dev]")
# ============================================================================

COMPOSE_FILE := docker-compose.test.yml
TEST_BASE_URL := http://localhost:8181
LOCAL_BASE_URL := http://localhost:8080

PYTEST_ARGS ?= -v

.PHONY: test test-local lint clean down help

# ── Full Docker-backed test run ───────────────────────────────────────────────

test: ## Build server image, run full test suite in Docker, tear down
	@echo "==> Building Mnemo server image and starting test stack..."
	docker compose -f $(COMPOSE_FILE) up -d --build
	@echo "==> Waiting for server to become healthy..."
	@for i in $$(seq 1 60); do \
		curl -sf $(TEST_BASE_URL)/healthz > /dev/null 2>&1 && break; \
		sleep 2; \
	done
	@curl -sf $(TEST_BASE_URL)/healthz > /dev/null 2>&1 || \
		(echo "ERROR: Mnemo server did not become healthy in time." && \
		 docker compose -f $(COMPOSE_FILE) logs test-mnemo && \
		 docker compose -f $(COMPOSE_FILE) down -v && exit 1)
	@echo "==> Server healthy. Running test suite..."
	MNEMO_BASE_URL=$(TEST_BASE_URL) python -m pytest tests/ $(PYTEST_ARGS); \
	EXIT=$$?; \
	docker compose -f $(COMPOSE_FILE) down -v; \
	exit $$EXIT

# ── Local test run (no Docker) ────────────────────────────────────────────────

test-local: ## Run tests against a server already running on localhost:8080
	@echo "==> Running SDK tests against $(LOCAL_BASE_URL) ..."
	MNEMO_BASE_URL=$(LOCAL_BASE_URL) python -m pytest tests/ $(PYTEST_ARGS)

# ── Code quality ──────────────────────────────────────────────────────────────

lint: ## Run ruff (linter) and mypy (type checker) if available
	@command -v ruff   > /dev/null && ruff check mnemo/ tests/ || true
	@command -v mypy   > /dev/null && mypy mnemo/ || true

# ── Housekeeping ──────────────────────────────────────────────────────────────

clean: ## Remove compiled Python artefacts
	find . -type d -name __pycache__ -exec rm -rf {} + 2>/dev/null || true
	find . -type f -name "*.pyc" -delete 2>/dev/null || true

down: ## Tear down the test Docker Compose stack
	docker compose -f $(COMPOSE_FILE) down -v

# ── Help ──────────────────────────────────────────────────────────────────────

help: ## Show this help message
	@grep -E '^[a-zA-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) | \
		awk 'BEGIN {FS = ":.*?## "}; {printf "  \033[36m%-14s\033[0m %s\n", $$1, $$2}'
