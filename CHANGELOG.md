# Changelog

All notable changes to this project are documented in this file. Entries follow the
[GitHub releases](https://github.com/muhib-karim/openclaw-desktop/releases) of this repository.

## [Unreleased]

- A v0.2.0 release is prepared as a draft and has not been published yet.
- Changes on `master` since v0.1.0: see the [comparison](https://github.com/muhib-karim/openclaw-desktop/compare/v0.1.0...master).

### Fixed

- Windows: **Install OpenClaw** failed with "program not found" because `npm` is `npm.cmd`.
- Windows: console windows no longer flash when the app checks status, runs diagnostics or doctor,
  or stops/restarts the gateway.
- macOS and Linux: OpenClaw, npm and Node are now found when the app is started from Finder or a
  desktop launcher instead of a terminal.
- The window no longer freezes while `openclaw doctor`, `npm install` or a status check is running.
- **Run Doctor** output no longer disappears a second after it is shown.
- The app no longer hangs on "Checking installation..." or "Starting..." when something goes wrong;
  failures are shown in the window with a way to retry.
- A second gateway process is no longer started while the first is still coming up.
- The tray menu shows the live gateway status instead of the status at launch.
- The log panel reads only the end of the log, tolerates non-UTF-8 output, and includes
  `gateway_error.log`.

### Security

- The gateway token is no longer passed through the webview for **Open in browser**.
- Tighter Content Security Policy and webview permissions (`opener:default` removed).
- Log files are created with owner-only permissions on macOS and Linux.

### Changed

- Removed the Ko-fi support links from the app, the README and the release notes template.
- pnpm is the only supported package manager (`bun.lock` removed); dev tooling refreshed.
- CI: ESLint, a Windows Rust job, a gitleaks scan, pinned actions, timeouts and Dependabot.

## [0.1.0] - 2026-02-17

[Release page](https://github.com/muhib-karim/openclaw-desktop/releases/tag/v0.1.0).

- Desktop builds for macOS Intel (`.dmg`, `.app.tar.gz`) and Linux (`.AppImage`, `.deb`, `.rpm`).
- No Windows or Apple Silicon build is attached to this release; use v0.0.2 for those.

## [0.0.2] - 2026-02-08

[Release page](https://github.com/muhib-karim/openclaw-desktop/releases/tag/v0.0.2).

- Desktop builds for Windows (`.msi`, setup `.exe`), macOS Intel and Apple Silicon (`.dmg`, `.app.tar.gz`) and Linux (`.AppImage`, `.deb`, `.rpm`).
