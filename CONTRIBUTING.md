# Contributing

## Workflow

1. Branch from `main`: `git checkout -b feat/<thing>`
2. Make changes. Run locally:
   ```bash
   cargo fmt --all
   cargo clippy --workspace --all-targets -- -D warnings
   cargo test --workspace --locked
   ```
3. Push branch and open PR against `main`.
4. CI must be green (build+test ubuntu/macos, clippy, fmt). `main` is protected — direct pushes blocked.
5. Squash merge.

## Standards

See `AGENTS.md` for crate dependency order, error-handling conventions, serde requirements, and clippy policy. See `CLAUDE.md` for allowed commands and workspace structure.

## Releases

Maintainer tags `v<major>.<minor>.<patch>` from green `main`. CI builds + uploads per-OS tarballs to the GitHub Release.
