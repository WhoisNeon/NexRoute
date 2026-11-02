# Changelog

All notable changes to NexRoute are documented here.

## [0.1.0] - 2026-11-02

### Added
- Initial release of NexRoute.
- Bulk import of `vless://`, `vmess://`, `ss://`, `socks://`, and `http://` proxy URIs.
- Subscription URL import.
- SQLite configuration storage with testing history.
- Real HTTP/HTTPS delay testing with configurable URL and timeout.
- Rotating proxy pool in internal and upstream exposure modes.
- sing-box and Xray-core sidecar support with selectable core.
- React + TypeScript dark-only UI.
- Multi-platform GitHub Actions release builds.

### Security
- All local listeners bind exclusively to `127.0.0.1`.

### Notes
- First release. Unsigned builds may trigger SmartScreen/Gatekeeper warnings.