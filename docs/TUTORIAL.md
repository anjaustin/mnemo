# Tutorial: Build a Support Agent with Mnemo Memory

This tutorial walks you through building a support agent that remembers customer
context across conversations. By the end, you'll have a working agent that:

- Stores customer interactions as memories
- Retrieves relevant context for new conversations
- Tracks how customer preferences change over time
- Uses the knowledge graph to find related entities

**Prerequisites**: Docker, Python 3.9+, an Anthropic or OpenAI API key.

**Time**: ~20 minutes.

---

## 1. Start Mnemo

```bash
git clone https://github.com/anjaustin/mnemo.git
cd mnemo

# Set your LLM key
export MNEMO_LLM_API_KEY=sk-ant-...

# Start the stack
docker compose up -d
```

Verify it's running:

```bash
curl http://localhost:8080/health
# {"status":"ok","version":"0.5.0"}
```

## 2. Install the Python SDK

```bash
pip install mnemo-client
```

## 3. Store customer interactions

```python
from mnemo import Mnemo

m = Mnemo("http://localhost:8080")

# Customer tells us about their setup
m.add("customer-42", "I'm running a Node.js app on AWS ECS with PostgreSQL", role="user")
m.add("customer-42", "We have about 50,000 daily active users", role="user")
m.add("customer-42", "I'm interested in migrating to serverless", role="user")

# A few days later, the customer updates their situation
m.add("customer-42", "We completed the serverless migration last week. Now using Lambda + DynamoDB", role="user")
m.add("customer-42", "Our daily active users grew to 75,000 after the migration", role="user")
```

Each `add()` call creates an **episode** that Mnemo processes asynchronously:
extracting entities, building relationships in the knowledge graph, and
embedding for semantic search.

## 4. Retrieve context for a new conversation

```python
# A support agent starts a new conversation
ctx = m.context("customer-42", "What infrastructure is this customer using?")

print(ctx.text)
# The context block will contain relevant facts assembled for an LLM prompt,
# prioritizing CURRENT facts (Lambda + DynamoDB) over superseded ones (ECS + PostgreSQL).

print(f"Token count: {ctx.token_count}")
print(f"Episodes used: {len(ctx.episodes)}")
print(f"Entities found: {len(ctx.entities)}")
```

Mnemo's temporal retrieval automatically understands that "What infrastructure
is this customer using?" implies **current** intent, so it boosts current facts
and deprioritizes superseded ones.

## 5. Explore the knowledge graph

```python
# See what entities were extracted
entities = m.graph_entities("customer-42")
for e in entities["data"]:
    print(f"  {e['name']} ({e['entity_type']}) - mentioned {e['mention_count']} times")

# Example output:
#   Node.js (Technology) - mentioned 1 times
#   AWS ECS (Service) - mentioned 1 times
#   PostgreSQL (Technology) - mentioned 1 times
#   Lambda (Service) - mentioned 1 times
#   DynamoDB (Service) - mentioned 1 times

# See relationships between entities
edges = m.graph_edges("customer-42", valid_only=True)
for e in edges["data"]:
    print(f"  {e['source_name']} --[{e['label']}]--> {e['target_name']}")
```

Notice that edges from `customer-42 -> AWS ECS` are now **invalidated**
(superseded by `customer-42 -> Lambda`), but still visible when you set
`valid_only=False` for historical analysis.

## 6. Track changes over time

```python
from datetime import datetime, timedelta, timezone

# What changed in the last 7 days?
week_ago = (datetime.now(timezone.utc) - timedelta(days=7)).isoformat()
now = datetime.now(timezone.utc).isoformat()

changes = m.changes_since("customer-42", from_dt=week_ago, to_dt=now)
print(f"New episodes: {changes['new_episodes']}")
print(f"New entities: {changes['new_entities']}")
print(f"Superseded facts: {changes['invalidated_edges']}")
```

## 7. Use temporal queries

```python
# What was true BEFORE the migration?
old_ctx = m.context(
    "customer-42",
    "What infrastructure is this customer using?",
    time_intent="historical"
)
print("Historical:", old_ctx.text)
# Will prioritize the ECS + PostgreSQL facts

# What is true NOW?
current_ctx = m.context(
    "customer-42",
    "What infrastructure is this customer using?",
    time_intent="current"
)
print("Current:", current_ctx.text)
# Will prioritize Lambda + DynamoDB
```

## 8. Set up webhooks (optional)

Get notified when new facts are extracted:

```python
webhook = m.create_webhook(
    url="https://your-app.com/webhooks/mnemo",
    events=["fact_added", "fact_superseded", "conflict_detected"],
    secret="your-hmac-secret"
)
print(f"Webhook ID: {webhook['id']}")

# Check delivery stats later
stats = m.get_webhook_stats(webhook["id"])
print(f"Delivered: {stats['delivered_events']}, Failed: {stats['failed_events']}")
```

## 9. Generate a memory digest

```python
# Get an LLM-generated summary of everything Mnemo knows about this customer
digest = m.memory_digest("customer-42")
print(digest["summary"])
print(f"Topics: {digest['dominant_topics']}")
```

The digest is generated during **sleep-time compute** (idle windows) or on
demand via this API call.

## 10. Use with LangChain (optional)

```python
from mnemo.ext.langchain import MnemoChatMessageHistory
from langchain_core.runnables.history import RunnableWithMessageHistory
from langchain_anthropic import ChatAnthropic

history = MnemoChatMessageHistory(
    base_url="http://localhost:8080",
    user="customer-42",
    session_id="support-session-1",
)

llm = ChatAnthropic(model="claude-sonnet-4-20250514")

chain_with_history = RunnableWithMessageHistory(
    llm,
    lambda session_id: MnemoChatMessageHistory(
        base_url="http://localhost:8080",
        user="customer-42",
        session_id=session_id,
    ),
)

response = chain_with_history.invoke(
    "What infrastructure changes has this customer made recently?",
    config={"configurable": {"session_id": "support-session-1"}},
)
print(response.content)
```

## 11. Open the operator dashboard

Navigate to **http://localhost:8080/_/** to see:

- **Home**: Health status, queue depth, recent activity
- **Explorer**: Visual knowledge graph browser
- **LLM Spans**: Token usage and latency for all LLM calls
- **Webhooks**: Delivery status, dead-letter queue, replay controls
- **Governance**: Per-user retention policies and audit trail
- **Time Travel**: Compare memory snapshots between two points in time

---

## Next steps

- [API Reference](API.md) - Full endpoint documentation
- [Architecture](ARCHITECTURE.md) - How Mnemo works under the hood
- [Python SDK Reference](../sdk/python/README.md) - All SDK methods
- [Webhook Guide](WEBHOOKS.md) - Event types and delivery model
- [Troubleshooting](TROUBLESHOOTING.md) - Common issues and fixes
