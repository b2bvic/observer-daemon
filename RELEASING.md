# Build a macOS release

You need Rust, Cargo, and the Xcode command line tools on arm64 macOS.
Run these commands from the repository root:

```bash
cargo test --locked --target aarch64-apple-darwin
cargo build --release --locked --target aarch64-apple-darwin
```

You get `target/aarch64-apple-darwin/release/observer-daemon`.
Configure a copy of `spec.toml.example` before you run the binary:

```bash
target/aarch64-apple-darwin/release/observer-daemon --config spec.toml --validate "The file is ready."
```

Verify the architecture and signature:

```bash
file target/aarch64-apple-darwin/release/observer-daemon
codesign --verify target/aarch64-apple-darwin/release/observer-daemon
```

The build uses an ad hoc signature. You do not get an Apple notarization ticket.
Validation reports a score. You must implement any action-blocking gate separately.
