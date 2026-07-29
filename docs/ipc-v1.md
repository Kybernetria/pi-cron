# pi-cron IPC version 1

All channels use UTF-8 JSON. Non-finite numbers and invalid UTF-8 are rejected. Prompts and protocol inputs are carried in stdin/socket data, never process arguments.

## Extension/client → daemon

The transport is a user-private Unix stream socket (`0600`). One request and one response are exchanged per connection, each as one newline-terminated JSON object. Requests are limited to 1,048,576 bytes; client responses are limited to 2,097,152 bytes.

Request envelope:

```json
{"id":"uuid","method":"add|edit|delete|list|get|enable|disable|run_now|ping","params":{}}
```

Response envelopes:

```json
{"id":"uuid","ok":true,"result":{}}
{"id":"uuid","ok":false,"error":{"code":"INVALID_REQUEST","message":"..."}}
```

`REQUEST_TOO_LARGE` is returned when possible for oversized input. A method has exactly one response. Disconnecting does not retry or cancel an already claimed occurrence.

## Rust daemon → TypeScript runner

The daemon directly spawns the runner in a new process group. It writes exactly one JSON document (maximum 1,048,576 bytes) to stdin and closes stdin. The runner writes exactly one newline-terminated response to stdout (maximum 131,072 bytes); diagnostics are stderr-only.

```json
{"version":1,"operation":"validate_protocol","action":{"type":"protocol","target":"node.provide","input":{}}}
{"version":1,"operation":"execute","job":{}}
```

```json
{"version":1,"ok":true,"result":{"valid":true}}
{"version":1,"ok":true,"result":{"status":"completed","result":"..."}}
{"version":1,"ok":false,"error":{"code":"RUNNER_FAILURE","message":"..."}}
```

Targets are exactly `node.provide`, where both components match `[a-z0-9][a-z0-9_-]*`; `pi_cron.*` is rejected to prevent control-plane recursion. Chat runners use the action's canonical cwd, while protocol runners use canonical `$HOME`. Project-local Pi resources are untrusted and therefore not loaded. Validation has a 60-second timeout. Execution defaults to 15 minutes and is configurable with `PI_CRON_RUNNER_TIMEOUT_MS`. Timeout, daemon shutdown, oversized output, bootstrap failure, signal, or nonzero exit kills the complete runner process group and becomes a terminal occurrence error. No runner invocation is retried. Ephemeral protocol and Pi SDK sessions are disposed before runner exit.

Result and error text is limited to 40,000 characters. Trace summaries retain terminal trace/span provenance events.

## Terminal state and compatibility

Occurrence statuses remain `claimed`, `completed`, and `skipped`. Claims interrupted by restart become `skipped`; external failures are terminal `completed` records with `error`. History retains the newest 100 rows per job. Scheduling is serialized, uses five-field cron in an IANA timezone, and neither retries nor catches up missed occurrences.
