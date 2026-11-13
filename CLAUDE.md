# Universal Clipboard

Clipboard sync between Windows, macOS, Linux, iOS and Android. Rust workspace in `crates/`.
Plan and spec: `docs/plan/2026-11-13-spec-and-plan.md` (read sections 5 to 7 first).
Desktop design (locked): `docs/design/desktop`.
Core library choices (M1): `docs/core.md`.
Pairing and membership protocol (signed off, M3 implements it word for word): `docs/protocol.md`.
Pairing, membership and key store code notes (M3): `docs/pairing.md`.

## Rules
- Code goes on a branch and a PR. Never push code to main. `docs/*.md` may go to main.
- Tests run headless. Never touch the real clipboard; use the fake clipboard and `UCLIP_PROFILE=dev`.
- Rust copies and worktrees share ONE `CARGO_TARGET_DIR`.
- No "send" or "get" actions in the UI: the latest copy is on every device automatically.

## Build, test, release
- Typecheck/lint: `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings`
- Unit: `cargo nextest run --workspace --no-tests=pass` (or `cargo test --workspace`)
- E2E (headless): none yet    Run when: apps/desktop/**, crates/uc-agent/**
- Build / package: `cargo build --workspace`   Artifact: n/a (installer tool not chosen yet, ask first)
- Known failures to tolerate: none
- Version source: `Cargo.toml` (`workspace.package.version`)   Release: release.yml on push to main (not added yet)
- Install locally after merge: n/a (no app yet)   Confirm version: n/a
- Deploy: none
- Signing: unsigned
