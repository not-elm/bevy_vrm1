# Repository Guidelines

## Project Structure & Module Organization
- `src/` holds the Rust crate. VRM runtime lives under `src/vrm/`, VRMA animation under `src/vrma/`, and shared system helpers in `src/system_param/` and `src/system_set.rs`.
- `examples/` contains runnable demos such as `spring_bone.rs`, `look_at_cursor.rs`, and `vrma.rs`.
- `assets/` includes sample VRM/VRMA content used by examples and tests.
- `docs/` provides images and documentation assets referenced by the README.
- `CHANGELOG.md` tracks release notes and breaking changes.

## Build, Test, and Development Commands
- `cargo check` validates compilation quickly.
- `cargo build` builds the crate; use `cargo build --features serde,log` to enable optional features.
- `cargo test` runs unit tests (many live inline under `src/` with `#[test]`).
- `cargo test --features log` enables logging in tests.
- `cargo run --example spring_bone` (or other examples) runs demos for visual verification.
- `cargo clippy` runs linting (Clippy lints are configured in `Cargo.toml`).
- `cargo fmt` formats code using `rustfmt.toml`.

## Coding Style & Naming Conventions
- Follow standard Rust style: 4-space indentation, `snake_case` for functions/modules, `CamelCase` for types, and `SCREAMING_SNAKE_CASE` for constants.
- Keep public API names aligned with VRM spec terms (`SpringBone`, `LookAt`, `NodeConstraint`).
- Prefer small, focused systems and keep Bevy system ordering explicit in `VrmSystemSets`.

## Testing Guidelines
- Tests are co-located with implementation (use `#[test]` modules in `src/`).
- Use `bevy_test_helper` when a minimal Bevy app is required.
- Name tests after behavior (e.g., `test_look_at_cursor_updates`).

## Commit & Pull Request Guidelines
- Commit messages follow a lightweight conventional style (examples: `feat: ...`, `fix: ...`, `chore: ...`, `update: ...`).
- Update `CHANGELOG.md` for user-facing changes or releases.
- PRs should describe behavior changes, include example commands run, and note any spec-related impacts.

## Architecture & Spec Notes
- System ordering must match the VRM update order; refer to `VrmSystemSets` and the manual transform propagation points.
- When changing behavior tied to VRM specs, link the relevant spec section in the PR description.
