# AGENTS.md

## Project
NexRoute is a Tauri v2 desktop application (Rust backend, React frontend) that imports proxy URIs/subscriptions, measures real HTTP latency, and exposes an explicit localhost-only proxy gateway.

## Architecture
- `src/` — React + TypeScript + custom CSS frontend.
- `src-tauri/src/config/` — URI parsing, subscription handling, models, SQLite persistence.
- `src-tauri/src/core/` — sidecar lifecycle, configuration generation, API clients.
- `src-tauri/src/pool/` — rotation strategy, health tracking, connection management.
- `src-tauri/src/proxy/` — local HTTP and SOCKS5 listeners.
- `src-tauri/src/commands/` — Tauri IPC commands (in `commands.rs`).

## Coding standards
- Frontend TypeScript strict mode; no unused locals/parameters.
- Rust: use `AppResult`, structured errors, never unwrap in commands.
- All listeners must bind `127.0.0.1`, never `0.0.0.0`.
- Pool starts only on explicit user action.

## Security
- Localhost-only listeners and sidecars.
- No telemetry, cloud sync, or accounts.
- Never implement TUN, transparent proxy, or system proxy changes.
- Validate all URIs and settings at trust boundaries.

## Testing and CI restrictions
- Never install the Rust toolchain locally.
- Never run `cargo build`, `cargo test`, `tauri build`, or any Rust compilation locally.
- Frontend checks allowed: `npm run typecheck`, `npm run build`.
- Rust tests run only in GitHub Actions.

## Sidecar management
- Sidecars are fetched via `npm run sidecars` using `scripts/fetch-sidecars.mjs`.
- Binaries are not committed.
- Tauri `externalBin` names follow target-triple suffix requirements.

## Release and versioning
- Update `CHANGELOG.md` and `RELEASE_NOTE.md` before tagging.
- Tag format: `vX.Y.Z`.
- Workflow `.github/workflows/release.yml` runs on tag push or manual dispatch.
- Builds produce draft GitHub Release with artifacts.

## Local Rust prohibition
Agents must not compile or test Rust locally under any circumstances. Use GitHub Actions for all Rust validation and packaging.