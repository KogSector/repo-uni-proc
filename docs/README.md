# Repository Unified Processor (repo-uni-proc)

**Port**: 8090 · **Role**: Code analysis, chunking and same-source
knowledge-graph construction for repositories

> This is the canonical README for the service; **all documentation lives in
> [`docs/`](./)** (this folder). See [index.md](index.md) for the docs index.

## Overview

`repo-uni-proc` processes **code repositories** for the ConFuse platform under
a **Strict Same-Source Policy**: it processes one source at a time and only
extracts relationships within that single source (e.g. function calls within
the repo).

- **Code Analysis**: Parse, normalize and chunk code across languages
- **Language Detection**: Automatic programming-language detection
- **Intelligent Chunking**: AST-aware chunking ("islands" of knowledge)
- **Metadata Extraction**: Symbols, imports, type references
- **Kafka Integration**: Publish chunk events to `embeddings-service`
- **Graph Storage**: Write nodes and structural edges directly to FalkorDB

*Note: Cross-source relationship discovery is handled asynchronously by the
`fast-fetcher` microservice.*

> The document flavour lives in `doc-uni-proc` (document extraction/chunking).

## Key API surface

| Endpoint | Method | Description |
|----------|--------|-------------|
| `/health` | GET | Health check |
| `/api/v1/codebase/analyze` | POST | Analyze code content (auth required) |
| `/api/v1/codebase/batch` | POST | Batch analyze |
| `/api/v1/codebase/languages` | GET | Supported languages |
| `/api/v1/codebase/metrics` | POST | Code metrics |
| `/api/v1/chunk` | POST | Legacy alias of analyze |
| `/api/v1/process` | POST | Legacy compatibility endpoint |
| `/api/v1/status/{source_id}` | GET | Processing status |
| `/api/v1/graph/sync` | POST | Trigger graph sync (auth required) |

Full schemas: [api-reference.md](api-reference.md).

## How to run the microservice

### Prerequisites

- Rust 1.70+ · FalkorDB (Redis protocol) · Kafka

### Installation

```bash
cargo build --release
```

### Configuration

```bash
export DATABASE_URL="postgresql://localhost/unified_processor"
export KAFKA_BOOTSTRAP_SERVERS="localhost:9092"
export FALKORDB_HOST="localhost"
export FALKORDB_PORT="6379"
export FALKORDB_PASSWORD="your-password"
export AUTH_MIDDLEWARE_URL="http://localhost:3010"
export PORT=8090
```

## Testing

Tests for this service live in the central test module (this service contains
no tests):

```bash
cd ../ConFuse-test-module
pip install -r requirements.txt
pytest unified_processor -v
```

## Documentation

| Document | Contents |
|----------|----------|
| [index.md](index.md) | Overview & docs index |
| [api-reference.md](api-reference.md) | Full endpoint reference |
| [ARCHITECTURE.md](ARCHITECTURE.md) | SourceRelationshipRouter & graph design |
| [setup.md](setup.md) | Local development setup |
