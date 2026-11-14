# Universal Clipboard

Clipboard sync between Windows, macOS, Linux, iOS and Android. Rust workspace in `crates/`.
Plan and spec: `docs/plan/2026-11-13-spec-and-plan.md` (read sections 5 to 7 first).
Desktop design (locked): `docs/design/desktop`. Desktop window (M8b): `docs/desktop.md`.
Core library choices (M1): `docs/core.md`.
Networking choices (M2, plain QUIC, LAN only): `docs/net.md`. Spike S1 so far: `docs/spikes/s1-interfaces.md`.
Windows clipboard backend and `tools/winhost` (M4): `docs/windows.md`.
Linux clipboard backends (M6), live tests in Docker: `docs/linux.md`.
Pairing and membership protocol (signed off, M3 implements it word for word): `docs/protocol.md`.
Pairing, membership and key store code notes (M3): `docs/pairing.md`.
macOS clipboard backend (M5): `docs/clip-mac.md`.
Agent, IPC and the `uc` CLI (M7), test switches, measurements: `docs/agent.md`.
Installers, firewall helper, self-update, keys and releases (M9): `docs/install.md`.

## Rules
- Code goes on a branch and a PR. Never push code to main. `docs/*.md` may go to main.
- Tests run headless. Never touch the real clipboard; use the fake clipboard and `UCLIP_PROFILE=dev`.
- Rust copies and worktrees share ONE `CARGO_TARGET_DIR`.
- No "send" or "get" actions in the UI or CLI: the latest copy is on every device automatically.
- The `uc` CLI is a full user interface: every settings-window action must also work from `uc` (plan 3.2).

## Build, test, release
- Typecheck/lint: `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings`
- Unit: `cargo nextest run --workspace --no-tests=pass` (or `cargo test --workspace`)
- Network lab (plan 6.3, Linux root, WSL): `wsl -d Ubuntu -u root -- bash tools/netsim/run-wsl.sh`
- Real Windows clipboard tests: `crates/uc-clip-win/tests/real.rs`, run by nextest inside a private window station (winhost).
- E2E (headless): multi-agent tests `cargo nextest run -p uc-cli` (real agents on loopback, paired through `uc`; one in winhost on Windows; `tests/update.rs` = N, broken N+1, rollback). Linux live agents: `sh crates/uc-clip-linux/live/docker.sh`. Cross-OS runner `tools/e2e/cross-os.ps1` is PENDING (no lab VMs). Run when: apps/desktop/**, crates/uc-agent/**, crates/uc-cli/**
- Desktop UI: in `apps/desktop`: `npm ci && npm run check && npm test && npm run e2e` (headless; screenshots compared with the design, 1%). Shell: `cd apps/desktop/src-tauri && cargo test` and `npx tauri build --no-bundle` (never run it on screen). The shell is not in the root workspace.
- Build / package: `cargo build --release -p uc-agent -p uc-cli`, then `pwsh packaging/windows/build.ps1 -Version X -Bin <target>/release -Out dist` (NSIS + MSIX), `sh packaging/macos/build-app.sh X <bin> dist` (on the Mac: `SKIP_DMG=1`), `wsl -d Ubuntu -- sh packaging/linux/wsl-build.sh X` (.deb, built `--no-default-features`)   Artifact: `dist/` (`*-setup.exe`, `*.msix`, `*.dmg`, `*.app.tar.gz`, `*.deb`)
- Install tests: CI (`package.yml`: `packaging/windows/test-install.ps1`, DMG mount check) and Docker for the .deb (`packaging/linux/test-deb.sh`). Install on Max's PC only as the install-before-merge step.
- Known failures to tolerate: none
- Version source: `Cargo.toml` (`workspace.package.version`)   Release: push a tag `vX.Y.Z` (must match Cargo.toml): `release.yml` builds, waits for approval in environment `release`, signs `latest.json` at 10%, makes a DRAFT release. `rollout.yml` raises the rollout.
- Install locally before merge: build NSIS with the window (`Universal Clipboard.exe` from `apps/desktop` in -Bin), run `<setup.exe> /S`   Confirm version: `uc --version`, `uc status --json` (`version`)
- Deploy: none
- Signing: OS signing none yet (Windows exe unsigned, macOS ad-hoc, notarization steps in `docs/install.md`). Update manifests: minisign-format updater key, secret outside the repo (`~/.config/uclip-release/updater.key`, and the `release` environment secret `UCLIP_UPDATER_KEY`).
