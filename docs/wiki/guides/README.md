# Guides

How-to guides for specific features and use cases.

---

## Integration Guides

| Guide | Description |
|-------|-------------|
| **[MCP Server](mcp-server.md)** | Integrate Mnemo with Claude, GPT, and other MCP-compatible agents |

---

## Existing Documentation

For detailed guides, see these existing docs:

| Topic | Documentation |
|-------|---------------|
| **Full Feature List** | [Capabilities](../../CAPABILITIES.md) |
| **API Usage Examples** | [Usage Guide](../../USAGE.md) |
| **Multi-Modal Memory** | [Multi-Modal PRD](../../MULTI_MODAL_PRD.md) |
| **Agent Identity** | [Agent Identity Substrate](../../AGENT_IDENTITY_SUBSTRATE.md) |
| **Webhooks** | [Webhooks Guide](../../WEBHOOKS.md) |
| **Security** | [Security Controls](../../SECURITY_CONTROLS.md) |

---

## Quick Examples

### Store and Retrieve

```python
client.add("user", "Important information")
ctx = client.context("user", "What do you know?")
```

### Query Historical State

```python
ctx = client.context("user", "What was true?", as_of="2025-01-15")
```

### Filter by Modality

```python
ctx = client.context("user", "Show images", include_modalities=["image"])
```

### Use a Memory Contract

```python
ctx = client.context("user", "Current state only", contract="current_strict")
```

---

## Architecture Reference

For understanding how features work internally:

- **[Architecture](../reference/architecture.md)** - System overview and pipelines
- **[Configuration](../reference/configuration.md)** - All settings
