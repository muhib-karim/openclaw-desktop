# AGENTS.md

Guidance for coding agents working in `OpenClaw-Desktop`.

## Project Snapshot

- Stack: Tauri 2 + React 19 + TypeScript + Vite + Rust.
- Frontend source: `src/`.
- Tauri/Rust source: `src-tauri/src/`.
- Main Rust commands exposed to frontend via `#[tauri::command]` in `src-tauri/src/lib.rs`.
- Package manager in active use: `pnpm` (lockfile exists). `bun` is also mentioned in README.
- CI workflows: `.github/workflows/ci.yml` (typecheck, Vitest, build, cargo fmt/clippy/test on push/PR) and `.github/workflows/build.yml` (tag-triggered release builds).

## Local Setup

- Node.js 18+ (CI uses Node 20).
- Rust stable toolchain.
- `pnpm` installed globally.
- Install deps: `pnpm install`.
- Tauri dev run (full app): `pnpm tauri dev`.

## Build / Run Commands

### Frontend

- Dev server only: `pnpm dev`.
- Production frontend build: `pnpm build`.
  - Runs `tsc && vite build`.
- Preview built frontend: `pnpm preview`.

### Desktop App (Tauri)

- Run desktop app in dev: `pnpm tauri dev`.
- Build installer/bundles: `pnpm tauri build`.
- Build for a specific Rust target:
  - `pnpm tauri build -- --target x86_64-pc-windows-msvc`
  - `pnpm tauri build -- --target x86_64-unknown-linux-gnu`
  - `pnpm tauri build -- --target x86_64-apple-darwin`

## Lint / Typecheck / Quality Commands

There is no dedicated `lint` script in `package.json` currently.

- Preferred TS correctness gate: `pnpm exec tsc --noEmit`.
- Equivalent full frontend gate (includes bundling): `pnpm build`.
- Rust formatting check: `cargo fmt --all -- --check` (run in `src-tauri/`).
- Rust lint: `cargo clippy --all-targets --all-features -- -D warnings` (run in `src-tauri/`).

If you add lint tooling (ESLint/Prettier/Biome), also add scripts in `package.json` and update this file.

## Test Commands

Current repository status:

- Rust unit tests live in the `tests` module of `src-tauri/src/lib.rs`.
- Frontend unit tests use Vitest (`src/**/*.test.ts`).

### Run all tests

- Frontend tests: `pnpm test`.
- Rust tests: `cargo test` (run in `src-tauri/`; requires `pnpm build` first so `../dist` exists).

### Run a single test (important)

- Rust single test by name: `cargo test test_name`.
- Rust single test by module path: `cargo test module_name::test_name`.
- Rust single integration test file: `cargo test --test file_name`.
- With output: `cargo test test_name -- --nocapture`.

### Frontend tests (Vitest)

- Watch mode: `pnpm test:watch`.
- Single test file: `pnpm exec vitest run src/lib/gateway.test.ts`.
- Single test name: `pnpm exec vitest run -t "test name"`.

## Coding Conventions

Follow existing code style before introducing new patterns.

### TypeScript / React

- Use strict TypeScript patterns; `tsconfig.json` has `strict: true`.
- Avoid `any`; prefer explicit interfaces/types (`GatewayStatus`, union types like `Page`).
- Keep components functional and hook-based.
- Keep side effects in `useEffect`; clean up intervals/timeouts.
- Use async/await for Tauri `invoke` calls.
- Handle all async failures with `try/catch` and user-safe fallbacks.
- Keep UI state local unless sharing is required.
- Prefer descriptive booleans: `installing`, `startingGateway`, `dashboardOpened`.
- Use `camelCase` for variables/functions, `PascalCase` for components/types.
- Keep imports grouped and readable:
  - React imports first.
  - Third-party packages next.
  - Local files last.
- Use double quotes and semicolons (matches current files).
- Preserve trailing commas where formatter/style currently uses them.

### Rust (Tauri backend)

- Keep Tauri commands small and focused.
- Return `Result<_, String>` for command errors exposed to frontend.
- Convert IO/process errors with contextual messages via `map_err`.
- Prefer early returns for guard conditions.
- Keep platform-specific logic behind `#[cfg(target_os = "windows")]` and non-Windows blocks.
- Use `snake_case` for functions/variables, `PascalCase` for structs/enums.
- Keep command registration centralized in `tauri::generate_handler![...]`.
- Avoid `unwrap()` in recoverable paths.
  - Existing `unwrap()` calls may remain unless touching related logic.
  - For new code, prefer propagation or explicit handling.
- Preserve tray behavior and window lifecycle semantics when editing `run()`.

### CSS / Styling

- Reuse CSS variables in `:root` (`--bg-*`, `--accent`, `--text-*`, etc.).
- Keep class names lowercase kebab-case.
- Prefer extending existing style blocks over introducing parallel style systems.
- Keep animations subtle and purposeful.

## Error Handling Expectations

- Never swallow errors silently.
- Frontend: log technical details with `console.error`, show user-safe message/state.
- Backend: return explicit error strings from commands for frontend handling.
- For process execution, check `status.success()` and surface stderr when useful.
- For filesystem access, handle missing paths and permission failures gracefully.

## Agent Workflow Expectations

- Make minimal, targeted diffs.
- Do not refactor unrelated areas in the same change.
- Update docs when changing commands, scripts, or development flow.
- If adding new commands, ensure frontend and Rust sides stay aligned.
- Run relevant checks before finishing:
  - Frontend changes: `pnpm build` (or at least `pnpm exec tsc --noEmit`).
  - Rust changes: `cargo test` and ideally `cargo clippy` in `src-tauri/`.

## Repository-Specific Notes

- Tauri config defines:
  - `beforeDevCommand`: `pnpm dev`
  - `beforeBuildCommand`: `pnpm build`
  - frontend dist output: `../dist`
- Vite dev server is expected on port `1420` (`vite.config.ts`).
- Gateway integration targets `127.0.0.1:18789`.
- CI builds on Windows, macOS (Intel + Apple Silicon), and Linux.

## Cursor / Copilot Rules

- No `.cursorrules` file found.
- No `.cursor/rules/` directory found.
- No `.github/copilot-instructions.md` file found.
- If these files are added later, mirror key constraints into this document.

## Safe Change Boundaries

- Do not change app identifier, signing, or bundle settings unless task requires it.
- Do not weaken Tauri security policy (`app.security.csp`) without explicit requirement.
- Do not change gateway port or command names without updating both frontend and backend.

## Definition of Done (per change)

- Code compiles for the touched layer(s).
- Relevant tests/checks pass or failures are explained.
- Behavior is validated for both success and failure paths.
- Docs (`AGENTS.md`, README, or inline notes) are updated when workflow changes.
