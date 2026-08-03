# Ripple Live Read-Only Tool Skills Design

## Summary

Ripple Live will add three useful read-only tools without introducing MCP or embedding provider-specific code in the Agent Gateway:

- `web_search` searches the public web through Tavily Search.
- `web_fetch` extracts readable content from one public URL through Tavily Extract.
- `weather_lookup` resolves a place and retrieves current and forecast weather through QWeather.

The tools are registered through the existing `skills/*/SKILL.md` and `tools.json` mechanism. One standalone Rust binary, `ripple-tool`, implements all three tools as subcommands. The Gateway remains responsible for tool discovery, process isolation, cancellation, generic validation, output limits, and the bounded Agent loop.

This is the first stage of a daily voice assistant. Calendar access, write actions, arbitrary MCP servers, and hot reload are out of scope.

## Goals

1. Replace the current DuckDuckGo Instant Answer implementation with useful general web search.
2. Let the Agent read a selected web page when search snippets are insufficient.
3. Answer Chinese current-weather and forecast questions with structured data.
4. Keep external-provider integrations outside the Gateway process.
5. Preserve source URLs, provider request identifiers, latency, cache, and error evidence.
6. Bound latency, paid API usage, result size, and tool-loop behavior.
7. Keep all operations strictly read-only.

## Non-Goals

- Calendar, email, messaging, reminders, tasks, or any other connector.
- Creating, updating, sending, or deleting external data.
- MCP client or server support.
- Skill hot reload.
- A general-purpose browser or direct access from `web_fetch` to private networks.
- Tavily-generated final answers; Qwen3-VL remains responsible for synthesis.
- A new mobile source-card UI in this phase.

## Existing System

The current Gateway already:

- sends OpenAI-compatible `tools` and `tool_choice` to Qwen3-VL;
- parses streamed and non-streamed tool calls;
- executes allowlisted native and Skill tools;
- returns tool output as a `role: "tool"` message;
- permits a bounded number of tool rounds;
- cancels a child process when a voice turn is interrupted;
- records tool arguments and results in the context event store; and
- emits `response.tool.started` and `response.tool.completed` events.

The current `web_search` is a native Rust branch backed by DuckDuckGo Instant Answer. It must be removed because an external Skill with the same name would otherwise produce duplicate schemas while native dispatch would continue to win.

## Chosen Architecture

```text
Voice / image / text
        |
        v
Qwen3-VL Agent loop
        |
        | structured tool call
        v
Gateway Skill Registry + CLI Runner
  - schema discovery
  - generic argument validation
  - env allowlist
  - timeout and cancellation
  - stdout/stderr limits
        |
        | JSON stdin / JSON stdout
        v
ripple-tool release binary
  - web-search       -> Tavily Search
  - web-fetch        -> Tavily Extract
  - weather-lookup   -> QWeather GeoAPI + Weather API
```

The standalone CLI is written in Rust but is not linked into the Gateway. Each tool invocation starts one short-lived process. This deliberately accepts process startup and lack of connection-pool reuse in exchange for provider isolation and simple deployment.

### Repository Layout

```text
tools/ripple-tool/
  Cargo.toml
  src/
    main.rs
    contract.rs
    cache.rs
    http.rs
    web.rs
    weather.rs

skills/web-research/
  SKILL.md
  tools.json

skills/weather/
  SKILL.md
  tools.json
```

`ripple-tool` uses subcommands rather than three binaries so authentication, HTTP behavior, retries, redaction, cache access, output truncation, and result envelopes have one implementation.

## Skill Registration

`skills/web-research/tools.json` registers `web_search` and `web_fetch`. `skills/weather/tools.json` registers `weather_lookup`. Each manifest command points to the deployed `ripple-tool` release binary plus its subcommand.

The manifests allow only the variables required by that provider and the shared cache:

- `RIPPLE_TAVILY_API_KEY`
- `RIPPLE_QWEATHER_API_HOST`
- `RIPPLE_QWEATHER_PROJECT_ID`
- `RIPPLE_QWEATHER_CREDENTIAL_ID`
- `RIPPLE_QWEATHER_PRIVATE_KEY_PATH`
- `RIPPLE_TOOL_CACHE_DB`
- `HTTPS_PROXY` when the deployment requires it

QWeather uses its recommended Ed25519 JWT flow. The CLI reads the private key from a server-side file, sets `kid` to the credential ID and `sub` to the project ID, and generates a short-lived token. The private key content is never placed in an environment variable, prompt, event, or tool result. QWeather also assigns a project-specific API host; legacy shared hosts are not used. See the official [QWeather authentication](https://dev.qweather.com/en/docs/configuration/authentication/) and [API host](https://dev.qweather.com/en/docs/configuration/api-host/) documentation.

Skills load at Gateway startup. Updating a manifest or replacing the CLI binary requires a Gateway restart in this phase.

## CLI Contract

Every subcommand:

1. reads exactly one JSON object from stdin;
2. validates types, enums, lengths, numeric ranges, and unknown fields;
3. writes exactly one JSON object to stdout;
4. writes no normal diagnostics to stdout;
5. never returns credentials, authorization headers, or raw provider headers; and
6. uses non-zero exit status only for a CLI crash, invalid runtime configuration, or a broken output contract.

Expected provider and user-input failures return exit status zero with `ok: false`, allowing the Agent to receive and explain a structured failure.

### Success Envelope

```json
{
  "ok": true,
  "data": {},
  "meta": {
    "provider": "tavily",
    "request_id": "provider-request-id",
    "elapsed_ms": 820,
    "cached": false
  }
}
```

### Error Envelope

```json
{
  "ok": false,
  "error": {
    "code": "UPSTREAM_TIMEOUT",
    "message": "搜索服务暂时没有响应",
    "retryable": true
  },
  "meta": {
    "provider": "tavily",
    "elapsed_ms": 12003
  }
}
```

Stable error codes are:

- `INVALID_ARGUMENT`
- `AUTH_MISSING`
- `UPSTREAM_TIMEOUT`
- `RATE_LIMITED`
- `UPSTREAM_ERROR`
- `NO_RESULTS`
- `CONTENT_BLOCKED`
- `AMBIGUOUS_LOCATION`

## Tool Definitions

### `web_search`

Input:

```json
{
  "query": "北京今天的重要科技新闻",
  "topic": "news",
  "time_range": "day",
  "max_results": 5
}
```

Rules:

- `query` is required, trimmed, non-empty, and limited to 200 Unicode characters.
- `topic` is `general` or `news`; the default is `general`.
- `time_range` is omitted or one of `day`, `week`, `month`, or `year`.
- `max_results` ranges from 1 to 8 and defaults to 5.
- Tavily `search_depth` is explicitly `basic`.
- Tavily `include_answer` and `include_raw_content` are false.

Output data:

```json
{
  "query": "北京今天的重要科技新闻",
  "result_count": 5,
  "results": [
    {
      "title": "...",
      "url": "https://...",
      "snippet": "...",
      "published_at": "...",
      "score": 0.87
    }
  ]
}
```

Each snippet is bounded before serialization. The CLI preserves Tavily's provider request ID and credit usage in `meta.request_id` and `meta.usage_credits`. Tavily Basic Search currently costs one credit and supports topic and time filtering; see the official [Search API](https://docs.tavily.com/documentation/api-reference/endpoint/search) and [credit pricing](https://docs.tavily.com/documentation/api-credits).

### `web_fetch`

Input:

```json
{
  "url": "https://example.com/article",
  "query": "提取文章中的产品发布时间"
}
```

Rules:

- `url` is required and must be an absolute HTTP or HTTPS URL.
- Credentials embedded in the URL are rejected.
- The hostname must not be localhost, a private IP literal, a link-local address, or another forbidden special-use address.
- `query` is optional, trimmed, and limited to 200 Unicode characters.
- Exactly one URL is extracted per call.
- Tavily Extract, rather than a Gateway-side HTTP fetch, performs the page retrieval.
- Returned content is cleaned Markdown or text and is truncated to about 12,000 Unicode characters.

Output data:

```json
{
  "url": "https://example.com/article",
  "content": "...",
  "content_chars": 11840,
  "truncated": true
}
```

The original URL is preserved. Provider failures, blocked content, and empty extraction are distinguished. Search with extraction is intentionally not used: the Agent first searches, selects at most two sources, then fetches only when snippets are insufficient. See Tavily's official [Extract guidance](https://docs.tavily.com/documentation/best-practices/best-practices-extract).

### `weather_lookup`

Input:

```json
{
  "location": "北京市朝阳区",
  "days": 3,
  "include_hourly": false
}
```

Rules:

- `location` is required, trimmed, and limited to 100 Unicode characters.
- `days` ranges from 1 to 7 and defaults to 3.
- `include_hourly` defaults to false and, when true, returns at most 24 hours.
- One CLI invocation performs place resolution followed by weather requests.
- A clearly ambiguous place returns `AMBIGUOUS_LOCATION` with a small candidate list instead of silently choosing.

Output data includes:

- resolved place name, administrative area, Location ID, coordinates, and timezone;
- observation time, temperature, feels-like temperature, condition, wind, humidity, and precipitation;
- up to seven daily forecasts;
- up to 24 hourly forecasts when requested; and
- QWeather source and attribution data.

The implementation uses QWeather GeoAPI for location resolution and QWeather current, daily, and optional hourly endpoints. See the official [GeoAPI](https://dev.qweather.com/en/docs/api/geoapi/) and [real-time weather](https://dev.qweather.com/en/docs/api/weather/weather-now/) documentation.

## Agent Calling Policy

- Requests containing explicit search, current news, latest information, or external facts use `web_search`.
- A user-supplied URL uses `web_fetch` directly.
- The Agent uses `web_fetch` after search only when snippets are insufficient.
- One answer may issue at most two searches and fetch at most two selected pages.
- Weather questions use `weather_lookup` directly and do not route through web search.
- A result with `ok: false` is evidence of tool failure, not external factual evidence.
- A zero-result search cannot be replaced with unsupported model knowledge presented as search output.
- The spoken answer names source sites briefly; full URLs remain in the structured tool result and event store.
- The existing six-round global tool-loop limit remains in place.

## Timeouts and Retries

| Tool | Per-attempt HTTP timeout | Retries after first attempt | Manifest process timeout |
| --- | ---: | ---: | ---: |
| `web_search` | 12 seconds | at most 2 | 40 seconds |
| `web_fetch` | 20 seconds | at most 1 | 45 seconds |
| `weather_lookup` | 10 seconds | at most 2 | 35 seconds |

The manifest timeout covers the complete CLI process, including all attempts and backoff, and stays below the registry's existing 300-second ceiling. The CLI stops starting new attempts when the remaining process budget cannot accommodate another request. It retries only network failures, HTTP 429, and selected HTTP 5xx responses. It does not retry authentication, authorization, schema, URL, or other HTTP 4xx failures. Retries use bounded exponential backoff with jitter and honor `Retry-After` when it fits within the process budget.

Turn cancellation drops and kills the child process. No retry continues after cancellation.

## Cache

The short-lived CLI processes share:

```text
.cache/agent-gateway/tool-cache.sqlite3
```

The database uses WAL mode and a short busy timeout. A cache key hashes the subcommand, contract version, provider, and canonical input; it never includes credentials.

| Data | TTL |
| --- | ---: |
| Search results | 5 minutes |
| Extracted page | 30 minutes |
| Place resolution | 24 hours |
| Current weather | 10 minutes |
| Weather forecast | 30 minutes |

Expired rows are ignored and lazily cleaned. Cache read/write failure falls back to a live request and is reported in diagnostic metadata; it does not fail the user request.

## Cost and Abuse Controls

- Search returns at most eight results and defaults to five.
- Search stays on one-credit Basic Search.
- One Agent response performs at most two searches and two page extractions.
- Tavily credit usage is recorded in tool-result metadata.
- Generic per-user, per-tool rate limits are applied before launching the CLI: 10 calls/minute for `web_search`, 10 calls/minute for `web_fetch`, and 20 calls/minute for `weather_lookup`. Exceeding a limit returns `RATE_LIMITED` without starting a process or spending provider credits.
- Tool output remains below the existing Gateway byte cap and also uses stricter per-field limits.
- No tool accepts arbitrary headers, provider parameters, command fragments, filesystem paths, or shell text from the model.

## Security

- Provider secrets remain only on the remote server.
- QWeather's Ed25519 private key is stored in a permission-restricted file outside the repository.
- The CLI never accepts a private-key path from model arguments.
- The CLI redacts known secrets and authorization values before producing any error.
- `web_fetch` relies on Tavily's remote extraction and validates the requested public URL before sending it.
- The Gateway continues to clear the child environment and passes only the manifest allowlist plus `PATH`.
- Tools are read-only by contract and implementation.
- Full extracted page content is untrusted data. The Agent prompt treats it as evidence, never as instructions that can override system or tool policy.

## Observability

Each invocation records:

- conversation/session and response identifiers at the Gateway layer;
- tool call ID and tool name;
- total elapsed time;
- cache hit or miss;
- provider request ID;
- result count or content length;
- provider credit usage when available;
- retry count; and
- stable error code.

Logs and events must not record API keys, JWTs, private keys, authorization headers, or full extracted documents. Existing `response.tool.started`, `response.tool.completed`, and `tool.result` events remain the integration points.

## Testing

### Rust Unit Tests

- subcommand parsing and single-JSON stdout behavior;
- complete input validation and rejection of unknown fields;
- provider response normalization;
- stable error mapping;
- URL validation and special-address rejection;
- result and content truncation;
- cache-key stability, TTL, and concurrent access;
- secret redaction; and
- QWeather JWT construction without exposing the signed token.

### Mock HTTP Tests

Use a local mock server to cover success, timeout, retryable 429/5xx, non-retryable 4xx, authentication failure, empty results, blocked extraction, malformed JSON, ambiguous place, and partial weather responses. Paid APIs are not used by the normal test suite.

### CLI Contract Tests

Run the built binary with JSON stdin and assert:

- stdout contains exactly one valid envelope;
- expected provider failures exit zero;
- configuration or process failures exit non-zero;
- stderr/stdout do not leak test credentials; and
- cancellation terminates promptly.

### Gateway Integration Tests

- all three schemas load from Skills;
- the old native DuckDuckGo schema and dispatch path are absent;
- invalid arguments fail before provider execution;
- timeouts, output limits, and cancellation remain enforced;
- tool results return to the model as `role: "tool"`; and
- mixed model text plus a tool call follows the already prepared compatibility behavior rather than failing the turn.

### Remote Smoke Tests

After deployment to `140.143.229.103`, run authenticated real-tool checks for:

1. a current-news query;
2. a Chinese long-tail general query;
3. extraction of one selected public page;
4. current and three-day Beijing weather;
5. an intentionally ambiguous location;
6. one controlled provider failure; and
7. the full voice path from speech through tool call and final TTS audio.

The smoke report must include tool name, provider request ID, cache state, elapsed time, result shape, final answer behavior, and whether any secret appeared in logs. HTTP 200 or a healthy Gateway alone is not acceptance.

## Deployment

`deploy/agent-stack/install.sh` builds `ripple-tool` in release mode and installs it at a stable absolute path used by the manifests. Deployment configuration supplies Tavily and QWeather credentials, the QWeather API host, the cache database path, and the existing outbound proxy where needed.

The deployment sequence is:

1. create provider projects and credentials;
2. place the QWeather private key outside the repository with restrictive permissions;
3. build the CLI and Gateway;
4. verify each CLI subcommand directly using JSON stdin;
5. restart the Gateway so it reloads Skill manifests;
6. confirm the expected external-tool count and schemas; and
7. run the remote smoke suite.

No credential is committed, printed in a command transcript, embedded in a manifest, or returned to the model.

## Deferred Work

- mobile source cards and expandable tool traces;
- calendar read access;
- write tools and approval/resume semantics;
- MCP adapters;
- Skill hot reload;
- per-user usage dashboards and billing limits; and
- provider fallback beyond Tavily and QWeather.
