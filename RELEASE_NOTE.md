# Release Notes — v0.1.0

NexRoute's first release: a local, explicit proxy gateway with real-delay testing and a rotating pool.

## Added
- Import proxy URIs and subscription URLs for VLESS, VMess, Shadowsocks, SOCKS5, and HTTP.
- Real HTTP round-trip delay testing with configurable URL and timeout.
- SQLite persistence of configurations and test results.
- Internal rotation mode (one local HTTP and one SOCKS5 listener) and upstream pool exposure mode.
- sing-box and Xray-core sidecar support, defaulting to sing-box.
- Dark-only desktop UI with configuration list, pool, dead-config review, and settings.

## Security
- All local listeners bind only to 127.0.0.1.
- No TUN, transparent proxy, or system proxy configuration.

## Notes
- Builds are unsigned; Windows SmartScreen and macOS Gatekeeper warnings are expected.