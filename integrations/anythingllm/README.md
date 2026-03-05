# Mnemo Vector DB Provider for AnythingLLM

Drop-in vector database provider that lets [AnythingLLM](https://github.com/Mintplex-Labs/anything-llm) use Mnemo as its vector store.

## Installation

1. Copy this directory into your AnythingLLM installation:

```bash
cp -r integrations/anythingllm/ \
  /path/to/anything-llm/server/utils/vectorDbProviders/mnemo/
```

2. Register the provider in `anything-llm/server/utils/helpers/index.js`:

```js
// In getVectorDbClass(), add this case:
case "mnemo":
  const { Mnemo } = require("../vectorDbProviders/mnemo");
  return new Mnemo();
```

3. Add the UI entry in `anything-llm/frontend/src/pages/GeneralSettings/VectorDatabase/index.jsx`:

```js
// In the VECTOR_DBS array, add:
{
  name: "Mnemo",
  value: "mnemo",
  logo: MnemoLogo,  // or use a placeholder
  options: (settings) => <MnemoDBOptions settings={settings} />,
  description: "Production memory infrastructure with temporal reasoning and governance.",
}
```

4. Set environment variables:

```bash
VECTOR_DB=mnemo
MNEMO_ENDPOINT=http://localhost:8080   # your Mnemo server
MNEMO_API_KEY=                          # optional, if auth is enabled
```

## How It Works

- Each AnythingLLM **workspace** maps to a Mnemo **namespace** (an isolated Qdrant collection).
- Documents are split, embedded by AnythingLLM's configured embedding engine, then stored via Mnemo's Raw Vector API.
- Similarity search queries are routed through Mnemo's vector search endpoints.
- Namespace lifecycle (create, delete, count) is fully managed.

## API Surface

The provider uses these Mnemo endpoints:

| Operation | Endpoint |
|-----------|----------|
| Upsert vectors | `POST /api/v1/vectors/:namespace` |
| Similarity search | `POST /api/v1/vectors/:namespace/query` |
| Delete vectors | `POST /api/v1/vectors/:namespace/delete` |
| Delete namespace | `DELETE /api/v1/vectors/:namespace` |
| Count vectors | `GET /api/v1/vectors/:namespace/count` |
| Check namespace | `GET /api/v1/vectors/:namespace/exists` |
| Health check | `GET /health` |

## Testing

With Mnemo running locally:

```bash
# Python API test (39 assertions)
python3 integrations/anythingllm/test_api.py

# Node.js provider test (requires Node.js)
node integrations/anythingllm/test.js
```

## Requirements

- Mnemo v0.3.1+ with Raw Vector API support
- AnythingLLM (any recent version with the `VectorDatabase` base class)
