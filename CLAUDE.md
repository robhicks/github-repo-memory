# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What This Is

An MCP (Model Context Protocol) server written in Rust that builds a knowledge graph of a GitHub organization's repositories, dependencies, languages, teams, and topics. It ingests data from the GitHub API into FalkorDB (a graph database) and exposes query/exploration tools over MCP's Streamable HTTP transport.

## Build & Run

```bash
cargo build                  # compile
cargo run                    # start the server (requires env vars and FalkorDB running)
cargo test                   # run all tests
cargo test --lib parsers     # run only manifest parser tests
cargo test --lib client      # run only FalkorDB param tests
```

**Infrastructure:** FalkorDB must be running. Start it with:
```bash
docker compose up -d
```

**Required env vars** (or `.env` file): `ECMEM_GITHUB_CLIENT_ID`, `ECMEM_GITHUB_CLIENT_SECRET`, `ECMEM_GITHUB_REDIRECT_URI`, `ECMEM_GITHUB_ORG`. See `src/config.rs` for all `ECMEM_*` variables and defaults.

## Architecture

The server has four layers that form a pipeline: **GitHub API → Ingestion → Graph DB → MCP Tools**.

### Ingestion Pipeline (`src/github/`)
- `client.rs` — Octocrab-based GitHub API client (repos, languages, topics, teams, file trees, file content)
- `ingest.rs` — `Ingester` crawls an org's repos and populates the graph. Supports full sync, incremental sync, and single-repo sync
- `parsers.rs` — Extracts dependencies from manifest files (Cargo.toml, package.json, go.mod, requirements.txt, pyproject.toml)

### Graph Layer (`src/graph/`)
- `client.rs` — `GraphClient` wraps FalkorDB's async connection in `Arc<Mutex<_>>` (not thread-safe natively). Uses `FalkorParam` enum for typed Cypher parameters with proper escaping
- `queries.rs` — All Cypher queries as constants. Write operations use `MERGE` for idempotency. Includes a `MERGE_CROSS_REPO_DEPENDENCY` query that links repos that depend on other repos in the same org
- `schema.rs` — Creates graph indices on startup (idempotent)
- `models.rs` — Rust structs for graph node types: Organization, Repository, Language, Dependency, Topic, Team, File

### Graph Schema (node types and relationships)
```
Organization -[HAS_REPO]-> Repository
Repository -[USES_LANGUAGE]-> Language
Repository -[HAS_DEPENDENCY]-> Dependency
Repository -[HAS_TOPIC]-> Topic
Repository -[OWNED_BY]-> Team
Repository -[HAS_FILE]-> File
Repository -[DEPENDS_ON_REPO]-> Repository  (cross-repo, resolved after sync)
```

### MCP Tools (`src/server.rs`, `src/tools/`)
- `server.rs` — `CodeMemoryServer` implements `ServerHandler` via rmcp macros (`#[tool_router]`, `#[tool_handler]`). Tool parameter structs use `schemars::JsonSchema` for auto-generated schemas
- `tools/search.rs` — search_repos, get_repo_details, find_dependents, find_related
- `tools/explore.rs` — explore_dependency_graph, list_languages, list_teams, get_org_stats

### Auth (`src/auth/`)
- MCP-native OAuth 2 flow with RFC 9728/8414 discovery (`.well-known` endpoints)
- `github_oauth.rs` — Full OAuth flow: authorize → GitHub callback → token exchange → dynamic client registration
- `middleware.rs` — Axum middleware that validates Bearer tokens. Supports two modes: `github` (token store lookup) and `jwt` (HS256 validation)
- `token_store.rs` — In-memory token cache with TTL (DashMap-based)

### Routing (`src/main.rs`)
OAuth routes are unauthenticated. The `/mcp` endpoint is behind `require_auth` middleware. Health check at `/health`.

## Key Patterns

- **FalkorDB parameter passing:** Use `FalkorParam` enum (String/Int/Bool) with the `into()` conversions. String values are auto-escaped for Cypher's single-quote format.
- **All graph writes use MERGE**, never CREATE, for idempotency.
- **rmcp tool macros:** Tools are defined with `#[rmcp::tool(description = "...")]` on methods in a `#[tool_router] impl` block. Parameter types must derive `Deserialize` + `JsonSchema`.
- **Config env prefix:** All environment variables use the `ECMEM_` prefix.
