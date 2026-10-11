# Architecture

One native Rust/Slint process normalizes manual and read-only LCU drafts, filters candidates and computes
a deterministic, explained ranking. Source epoch/session generation/revision checks reject stale updates.
Confirmed picks outrank hovers; enemy intent is ignored. Traits cover a curated subset, not all champions.

Bootstrap obtains Data Dragon labels and OP.GG MCP statistics using bounded requests/retries/deadlines.
Versioned cache has a 24-hour TTL and validated known filters. Partial failures preserve usable data;
unknown values are not fabricated. The statistical snapshot is frozen during an active draft;
ranking updates do not trigger external meta requests.

Native Windows process-image discovery finds the client lockfile; an explicit path is optional.
Dedicated Riot-root TLS uses fixed 127.0.0.1 HTTPS/WebSocket WAMP endpoints, without proxy, redirect
or insecure fallback. Events invalidate state for bounded REST resynchronization.
Credentials, names, PUUIDs, chat and raw sessions are not persisted.

Preferences and statistical cache are separate local files with atomic writes. Corrupt profiles are
preserved. Closing stops workers/connections; there is no service or autostart process.
See [synthetic fixture provenance](../tests/support/lcu/README.md).

Riot registration/audit and OP.GG distribution/data-use/cache/quotas are external requirements,
not established by tests. LCU compatibility and Windows game performance require independent validation.
