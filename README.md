<p align="center">
  <strong>NexRoute</strong><br />
  A local, explicit proxy gateway with real-delay testing and a rotating pool.
</p>

<p align="center">
  <img alt="License MIT" src="https://img.shields.io/badge/license-MIT-346b8d?style=flat-square" />
  <img alt="Tauri v2" src="https://img.shields.io/badge/tauri-v2-5b8def?style=flat-square" />
  <img alt="Rust" src="https://img.shields.io/badge/rust-backend-6c7a89?style=flat-square" />
</p>

---

NexRoute imports proxy configurations, measures real HTTP latency through each one, and exposes an explicit localhost-only proxy gateway. It never changes the system proxy, creates a TUN interface, or binds to anything other than `127.0.0.1`.

## Contents

- [Key features](#key-features)
- [Supported platforms](#supported-platforms)
- [Supported protocols](#supported-protocols)
- [Installation](#installation)
- [Usage](#usage)
- [Proxy modes](#proxy-modes)
- [Configuration storage and privacy](#configuration-storage-and-privacy)
- [Development](#development)
- [Releases](#releases)
- [Unsigned builds](#unsigned-builds)
- [License](#license)

## Key features

- Bulk-import `vless://`, `vmess://`, `ss://`, `socks://`, and `http://` URIs.
- Import subscription URLs that return supported proxy links.
- Normalize configurations into sing-box/Xray compatible outbounds.
- Real HTTP/HTTPS delay testing through temporary local inbounds.
- Persist configurations, latency, and status in SQLite.
- Maintain a separate dead/failed list with manual deletion.
- Expose internal rotating HTTP/SOCKS5 listeners or per-config listener pools.
- Bundle sing-box and Xray-core as Tauri sidecars; switch cores in settings.

## Supported platforms

- Windows (portable ZIP and NSIS installer)
- macOS (Apple Silicon and Intel DMG)
- Linux (DEB and tarball)

## Supported protocols

VLESS, VMess, Shadowsocks, SOCKS5, and HTTP. Protocol-specific validation prevents unsupported combinations (for example, Shadowsocks SIP003 plugins require sing-box).

## Installation

Download the latest release for your platform from the GitHub Releases page. Unsigned builds may require bypassing platform security warnings (see [Unsigned builds](#unsigned-builds)).

## Usage

1. Import URIs or a subscription URL.
2. Run Test All (or Test Selected).
3. Start the proxy pool in the desired mode from the Proxy Pool view.
4. Point your client at the displayed local endpoints.

Default test URL: `https://www.gstatic.com/generate_204`. Default timeout: 2000 ms.

## Proxy modes

**Mode A — Internal Rotation**
NexRoute rotates across alive configurations using round-robin or random selection and exposes one local HTTP port and one local SOCKS5 port.

**Mode B — Upstream Pool Exposure**
NexRoute exposes each alive configuration on its own HTTP/SOCKS5 listener pair so an external router can perform rotation. NexRoute does not perform upstream rotation in this mode.

Example endpoints:

- Internal: `http://127.0.0.1:8911`, `socks5://127.0.0.1:8912`
- Upstream: `http://127.0.0.1:<port>`, `socks5://127.0.0.1:<port>`

Ports are assigned dynamically; use the values shown in the application.

## Configuration storage and privacy

- Database location: platform application data directory (shown in Settings).
- Configurations remain local; there is no telemetry, cloud sync, or account.
- Listeners bind only to `127.0.0.1`.

## Development

Frontend and static checks can run locally:

```bash
npm install
npm run typecheck
npm run build
```

All Rust compilation, testing, and packaging must run through GitHub Actions. Do not install the Rust toolchain or run `cargo` locally. Sidecars are fetched in CI using:

```bash
npm run sidecars
```

## Releases

Release builds are produced by `.github/workflows/release.yml` on version tags (`v*`) or manual dispatch. Artifacts include Windows portable ZIP and NSIS installer, macOS Apple Silicon and Intel DMG, and Linux DEB and tarball.

## Unsigned builds

Release builds are not code-signed. On Windows, SmartScreen may warn about an unrecognized publisher. On macOS, Gatekeeper may block the first launch; use the system security settings to allow the app.

## License

NexRoute is released under the [MIT License](./LICENSE). Bundled third-party binaries (sing-box, Xray-core) remain under their own licenses.