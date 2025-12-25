# GEDCOM MCP Server Plan

## Objective
Implement a Model Context Protocol (MCP) server that serves GEDCOM data, supporting discoverability, querying, and updates.

## Status
- ✅ Project scaffolding: Rust binary crate with configuration loading and parsing tests.
- ✅ Protocol foundations (initial): JSON-RPC-style MCP request/response types, ping handler skeleton, and serialization unit tests.
- ✅ Protocol foundations (dispatch): raw message parsing with structured parse errors, handler dispatch, and additional serialization tests.
- ✅ Protocol foundations (line transport stub): JSON-per-line handler that dispatches and serializes outbound messages; tests cover happy path and parse errors.
- ✅ GEDCOM parsing layer: parser handles individuals/families with ID/name/husband/wife/children, birth/death events (date/place), errors on malformed input, and file loading helper.
- ✅ Query handlers: server can return individual and family details plus list endpoints, with validation and not-found coverage.
- ✅ Mutation handlers: create individuals/families with validation, conflict detection, and optional persistence hooks.
- ✅ Persistence & storage: optional JSON snapshot persistence on mutations; in-memory store is primary with snapshot load/save.
- 🚧 Observability & Tooling: env-filtered tracing with configurable log levels; add further metrics/hooks later.
- 🚧 Integration harness: CLI/config overrides for stdin/stdout server, default config path fallbacks.

## Iterative, Testable Steps
1. **Project Scaffolding** (done)
   - Set up Rust workspace and crate for the MCP server.
   - Add basic binaries and configuration loading.
   - **Tests:** Verify crate builds; unit test config parsing with sample fixtures.

2. **Protocol Foundations** (done)
   - Define core MCP message types and transport (e.g., JSON-RPC over stdio or TCP).
   - Implement request/response handling skeleton with tracing.
   - **Tests:** Unit tests for message serialization/deserialization and error handling. **Progress:** dispatch covers ping, unknown methods, parse errors, message serialization, line-based message handling, and stdio loop coverage.

3. **GEDCOM Parsing Layer** (done)
   - Introduce GEDCOM file parser or integrate an existing crate.
   - Map GEDCOM entities (individuals, families, events) into internal models.
   - **Tests:** Parser unit tests using small GEDCOM fixtures; verify error cases. **Progress:** parser handles individuals/families with ID/name/husband/wife/child fields, birth/death event date/place extraction, rejects orphan tags/missing IDs/invalid levels, and supports file loading helper.

4. **Query Handlers** (done)
   - Implement MCP endpoints to fetch GEDCOM entities, relationships, and metadata.
   - Support pagination/filters where applicable.
   - **Tests:** Handler unit tests with mocked data; contract tests for response shapes. **Progress:** `get_individual`, `get_family`, `list_individuals`, and `list_families` with validation, missing-store handling, and not-found coverage.

5. **Mutation Handlers** (done)
   - Support creating/updating GEDCOM records with validation.
   - Include conflict detection and consistent ID generation.
   - **Tests:** Unit tests for validation/mutations; ensure round-trip persistence. **Progress:** create individual/family handlers with validation, conflict detection, persistence hooks, and snapshot persistence tests.

6. **Persistence & Storage** (done)
   - Add storage abstraction (in-memory first, then file-backed).
   - **Tests:** Storage unit tests ensuring durability and consistency across reloads. **Progress:** optional JSON snapshot persistence when storage path is configured; in-memory store remains default, and server can load snapshots on startup.

7. **Observability & Tooling** (in progress)
   - Add structured logging, metrics hooks, and graceful shutdown.
   - **Tests:** Unit tests for logging hooks where feasible; integration smoke test for shutdown. **Progress:** env-filtered tracing with configurable log levels.

8. **Integration Harness** (in progress)
   - Provide CLI for serving a GEDCOM file and interacting via MCP tooling.
   - **Tests:** Integration test simulating end-to-end request flow with sample GEDCOM. **Progress:** CLI config path override and stdin/stdout serving loop.

9. **Documentation & Examples**
   - Document protocol mapping, configuration, and example workflows.
   - **Tests:** Ensure example snippets compile/run via doc-tests or CI scripts.
