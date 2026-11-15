# Crate layout

A Cargo workspace. The root `Cargo.toml` holds shared package metadata and the lint table; each
member lives in `crates/<name>/` and inherits both with `workspace = true`. Add a crate by adding a
directory under `crates/`.

- Library code lives under `src/`; keep `lib.rs` to module declarations and re-exports.
- Integration tests live in `tests/` and use only the public API. Unit tests sit beside the code in
  `#[cfg(test)]` modules.
- Lints are configured in the manifest (`[lints]`), not with `#![...]` attributes scattered in files.
