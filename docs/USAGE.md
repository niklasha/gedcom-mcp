# GEDCOM MCP Server Usage

## Quickstart
1. Copy the example config and adjust paths as needed:
   ```bash
   cp examples/config.example.toml config.toml
   ```
2. Ensure the GEDCOM file exists (a minimal sample is provided in `examples/sample.ged`).
3. Run the server, pointing to your config file (argument takes precedence over env):
   ```bash
   cargo run -- config.toml
   ```
   Or via environment variable:
   ```bash
   GEDCOM_MCP_CONFIG=config.toml cargo run
   ```

The server listens on the configured `bind_address` and communicates over stdin/stdout with MCP clients.

## Configuration fields
- `bind_address`: Socket address to advertise (e.g., `127.0.0.1:8080`).
- `gedcom_path`: Path to the GEDCOM input file.
- `persistence_path` (optional): JSON snapshot file for storing created/updated data; if present the server will attempt to load it on startup.

## Protocol overview
- Transport: newline-delimited JSON messages over stdin/stdout (one JSON object per line).
- Envelope:
  ```json
  { "id": "1", "method": "ping", "params": { ... } }
  ```
- Success response:
  ```json
  { "type": "response", "id": "1", "result": { ... } }
  ```
- Error response:
  ```json
  { "type": "error", "id": "1", "error": { "code": -32601, "message": "method not found" } }
  ```

## Common requests
- `ping`: Health check.
- `get_individual` / `get_family`: Fetch a record by ID.
- `list_individuals` / `list_families`: Enumerate stored records.
- `create_individual` / `create_family`: Add records (when persistence is configured, snapshots are saved automatically).

### Examples
- Get an individual:
  ```json
  {"id":"1","method":"get_individual","params":{"id":"I1"}}
  ```
- Create an individual:
  ```json
  {"id":"2","method":"create_individual","params":{"id":"I99","name":"New Person","birth":{"date":"1 JAN 1990","place":"Town"}}}
  ```
- List individuals:
  ```json
  {"id":"3","method":"list_individuals","params":{}}
  ```

## Error codes
- `-32601`: Method not found.
- `-32602`: Invalid params.
- `-32700`: Parse error.
- `-32000`: Server error (e.g., missing store).
- `-32001`: Conflict (duplicate).
- `-32004`: Not found.

## Persistence behavior
- If `persistence_path` is set, the server will:
  - Load from the snapshot at startup (falls back to GEDCOM source if load fails).
  - Write a JSON snapshot after successful `create_*` mutations using atomic rename.
