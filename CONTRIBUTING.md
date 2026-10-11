# Contributing

Use GitHub issues for reproducible bugs and proposals. Include app/OS versions, expected behavior
and sanitized steps. Never include tokens, client lockfiles, raw player captures or profile/cache files.

Use Rust 1.94.0 and the [README](README.md) test/build commands. Keep Cargo.lock committed.
Changes must pass core tests, gui-test, fmt and Clippy; new behavior needs regression tests.
Local TLS/WAMP tests bind loopback ports: sandbox restrictions are not product regressions.
Ignored native tests need a graphical environment and do not validate a live ranked client.

Use focused PRs and Conventional Commit titles (`feat:`, `fix:`, `test:`, `docs:`, `build:`, `ci:`).
Contributions to original code are under MIT. Preserve third-party notices.
Discuss new dependencies, telemetry, automatic game actions or resident services before implementing them.

## Windows releases

Build with `cargo build --release --locked --target x86_64-pc-windows-msvc --bin pikker`.
The workflows under `.github/workflows` define packaging/smoke-test commands.
Tags match Cargo's version; beta tags are prereleases, not stable-validation claims.
Publishing requires platform checks and resolved distribution-policy requirements.

## License notices

Install cargo-about 0.9.2 as a development tool and run:

```sh
sh scripts/update-notices.sh
```

Review choices and copyrights when Cargo.lock changes. The inventory targets Windows and excludes
dev dependencies, but build tooling may appear alongside linked dependencies.
