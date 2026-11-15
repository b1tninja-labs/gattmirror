# Instructions for coding agents

`gattmirror` is a Rust workspace (edition 2024, MSRV 1.85). Read `docs/layout.md` before adding modules.

## Commands

Run these from this directory; all four must pass before a change is done.

    cargo fmt --check
    cargo clippy --all-targets -- -D warnings
    cargo test
    cargo doc --no-deps

## Scope

`gattmirror` is transport only: it mirrors a BLE peripheral's GATT table and forwards attribute traffic
between the emulator's virtual controller and a real adapter. It must not parse OBD, ELM327 or UDS, and it
issues no vehicle writes. Read `PLAN.md` for the crates and `ROADMAP.md` for the obligations; rows are
`Status | Obligation | Anchor | Test`, per crate in `crates/<name>/ROADMAP.md`.

## Invariants

- First-party dependencies only (`std`; Microsoft's `windows` crate for the Windows client). Say why in the
  commit message before adding anything else.
- The Bluetooth Core Specification is not yet in `primarysource`. Do not cite a section or quote normative
  wording from memory: verify against a live controller and mark the row `[unread]`.
- No Bluetooth addresses, adapter names, VINs or local paths in the tree.
- `[ok]` needs a break: change the code, watch the named test fail, revert.

- `unsafe` is forbidden by the lint table in `Cargo.toml`; do not relax it to make something compile.
- Never run `cargo fmt` or `cargo clippy --fix` outside this directory.
- Do not raise `rust-version` without updating `clippy.toml`, `docs/msrv.md` and CI together.
- Do not add a dependency without saying why in the commit message.
- Write scratch output outside the repository; `git add -A` will commit it.
- A test that cannot fail is not a test: break the code once, watch it fail, revert.
- Commit messages carry no AI attribution trailer.
