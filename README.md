# Enterprise Code Memory

An MCP (Model Context Protocol) server that builds a knowledge graph of your GitHub organization's repositories, dependencies, languages, teams, and topics. It ingests data from the GitHub API into [FalkorDB](https://www.falkordb.com/) and exposes query and exploration tools over MCP's Streamable HTTP transport.

## Features

- **Organization-wide knowledge graph** — maps repos, dependencies, languages, topics, teams, and files
- **Cross-repo dependency resolution** — discovers when repos depend on other repos within the same org
- **Multi-org and GitHub Enterprise** — supports multiple orgs across github.com and GHE instances
- **Manifest parsing** — extracts dependencies from Cargo.toml, package.json, go.mod, requirements.txt, and pyproject.toml
- **MCP-native** — exposes tools via the Model Context Protocol for use by AI assistants
- **Dual transport** — Streamable HTTP for remote access, stdio for local MCP clients

## Prerequisites

- Rust (edition 2021+)
- Docker (for FalkorDB)
- A GitHub org to ingest
- `gh` CLI (if using `gh_cli` auth mode)

## Quick Start

**1. Start FalkorDB:**

```bash
docker compose up -d
```

**2. Configure environment variables** (or create a `.env` file):

```bash
# Minimal setup with gh CLI auth (easiest for local dev)
export ECMEM_AUTH_MODE=gh_cli
export ECMEM_GITHUB_ORG=my-org
```

**3. Run the server:**

```bash
cargo build
cargo run           # HTTP mode (default, serves at 127.0.0.1:8080)
cargo run -- --stdio  # Stdio mode (for MCP client integration)
```

## MCP Client Configuration

### Single Org (stdio)

```json
{
  "mcpServers": {
    "enterprise-code-memory": {
      "command": "enterprise-code-memory",
      "args": ["--stdio"],
      "env": {
        "ECMEM_AUTH_MODE": "gh_cli",
        "ECMEM_GITHUB_ORG": "my-org",
        "ECMEM_REPOS": "repo-a,repo-b"
      }
    }
  }
}
```

### Multiple Orgs / GitHub Enterprise (stdio)

```json
{
  "mcpServers": {
    "enterprise-code-memory": {
      "command": "enterprise-code-memory",
      "args": ["--stdio"],
      "env": {
        "ECMEM_AUTH_MODE": "gh_cli",
        "ECMEM_GITHUB_ORGS": "org-a@ghes1.corp.com,org-b@ghes2.corp.com,org-c",
        "ECMEM_REPOS": "org-a:repo-1,repo-2;org-b:repo-3"
      }
    }
  }
}
```

## MCP Tools

### Search

| Tool | Description |
|------|-------------|
| `search_repos` | Search repositories by name, language, topic, or dependency |
| `get_repo_details` | Get full details for a specific repository |
| `find_dependents` | Find repos that depend on a given package |
| `find_related` | Find repos related to a given repo by shared dependencies, languages, or topics |

### Explore

| Tool | Description |
|------|-------------|
| `explore_dependency_graph` | Traverse the dependency graph from a starting repo |
| `list_languages` | List all languages used across the org |
| `list_teams` | List all teams and their owned repos |
| `get_org_stats` | Get high-level statistics for the organization |

## Graph Schema

```
Organization -[HAS_REPO]-> Repository
Repository -[USES_LANGUAGE]-> Language
Repository -[HAS_DEPENDENCY]-> Dependency
Repository -[HAS_TOPIC]-> Topic
Repository -[OWNED_BY]-> Team
Repository -[HAS_FILE]-> File
Repository -[DEPENDS_ON_REPO]-> Repository  (cross-repo)
```

## Authentication

Three modes, selected via `ECMEM_AUTH_MODE`:

| Mode | Description | Use Case |
|------|-------------|----------|
| `gh_cli` | Uses `gh auth token` — no OAuth setup needed | Local development |
| `github` | MCP-native OAuth 2 with RFC 9728/8414 discovery | Production / remote clients |
| `jwt` | JWT Bearer token validation | Custom auth integrations |

## Environment Variables

All variables use the `ECMEM_` prefix.

| Variable | Default | Description |
|----------|---------|-------------|
| `ECMEM_AUTH_MODE` | `github` | Auth mode: `github`, `jwt`, or `gh_cli` |
| `ECMEM_GITHUB_ORG` | — | Single org name |
| `ECMEM_GITHUB_ORGS` | — | Multi-org: `org1@host1,org2@host2,org3` |
| `ECMEM_REPOS` | — | Restrict repos: `repo-a,repo-b` or `org:repo-a,repo-b;org2:repo-c` |
| `ECMEM_GITHUB_CLIENT_ID` | — | OAuth client ID (required for `github` mode) |
| `ECMEM_GITHUB_CLIENT_SECRET` | — | OAuth client secret (required for `github` mode) |
| `ECMEM_GITHUB_REDIRECT_URI` | — | OAuth redirect URI (required for `github` mode) |
| `ECMEM_GITHUB_API_URL` | `https://api.github.com` | GitHub API base URL (single-org mode) |
| `ECMEM_FALKORDB_HOST` | `127.0.0.1` | FalkorDB host |
| `ECMEM_FALKORDB_PORT` | `6379` | FalkorDB port |
| `ECMEM_FALKORDB_PASSWORD` | — | FalkorDB password |
| `ECMEM_FALKORDB_GRAPH` | `enterprise_code` | Graph name |
| `ECMEM_SERVER_HOST` | `127.0.0.1` | HTTP server bind address |
| `ECMEM_SERVER_PORT` | `8080` | HTTP server port |
| `ECMEM_JWT_SECRET` | — | JWT signing secret (required for `jwt` mode) |
| `ECMEM_SYNC_BATCH_SIZE` | `50` | Repos per sync batch |
| `ECMEM_MAX_FILE_DEPTH` | `2` | Max directory depth for file tree ingestion |

## Development

```bash
cargo build                  # Compile
cargo test                   # Run all tests
cargo test --lib parsers     # Manifest parser tests only
cargo test --lib client      # FalkorDB param tests only
```

## License

All rights reserved.
