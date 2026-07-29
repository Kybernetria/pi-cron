# Frozen compatibility contract

The Rust daemon preserves the version 1 behavior below:

- Socket paths, database paths, environment overrides, newline JSON envelopes, methods, and camel-case result shapes remain unchanged.
- SQLite schema version 1 is opened in place: `schema_version`, `jobs`, and `occurrences`, including existing indexes and action JSON. No destructive migration is performed.
- State/runtime directories are `0700`; database, WAL/SHM, lock, socket, and generated service files are `0600`. The systemd unit uses `UMask=0077`.
- Schedules are exactly five cron fields evaluated in an IANA timezone. `next_at` is strictly future. DST follows timezone civil-time behavior.
- Due occurrences are claimed independently of earlier long-running actions. Execution is globally serialized, every occurrence is persisted before effects, and unique `(job_id, scheduled_at)` prevents duplicate claims.
- There are no retries. Occurrences delayed by at least one minute are skipped rather than caught up. Claimed rows found at startup become skipped.
- External execution failures are terminal. Results/errors are at most 40,000 characters and each job retains at most 100 history rows.
- Add/edit validates the complete replacement before mutation. Protocol target existence and input schema are checked through the runner before commit.
- Chat runners use the action's canonical cwd. Protocol runners use canonical `$HOME`. Both use untrusted project scope and inherit the daemon environment.

The wire limits, timeout behavior, target grammar, and error envelopes are specified in [ipc-v1.md](./ipc-v1.md).
