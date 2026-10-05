# AI agent response validation: observer-daemon

Observer-daemon scores agent responses against configured writing rules for operators who review hosted-model output.
It records response violations so repeated failures remain visible across Claude Code and Codex CLI sessions.

[Project page](https://scalewithsearch.com/code/observer-daemon)

## Install

Use Rust 1.88 or newer and Cargo on macOS or Linux.

```bash
git clone https://github.com/b2bvic/observer-daemon.git
cd observer-daemon
cargo build --locked
```

For a reviewed source installation:

```bash
cargo install --path . --locked
```

## Quick start

Use a copied spec with isolated ledger paths:

```bash
mkdir -p .demo
sed 's|~/.observer/|./.demo/|g' spec.toml.example > .demo/spec.toml
cargo run --locked -- --config .demo/spec.toml --validate "The file is ready."
```

The sample text produces this result with the shipped example rules:

```text
Class: generic
Score: 100/100
No violations.
```

One-shot validation reads configured correction patterns. The isolated paths keep this example separate from an existing correction ledger.

## How it works

The validator classifies a prompt, selects its writing rubric, and applies configured deterministic checks.
Claude Code output checks and Codex response quality checks use supported JSONL message records.
In daemon mode, configured watch paths feed responses to the validator and its JSONL validation ledger.
Corrections and promoted rules retain their content class.
This supplies patterns a team can adopt for agent writing rule enforcement after it reviews the rules and action integration.

Configure existing transcript directories before starting the daemon:

```bash
cargo run --locked -- --daemon --config .demo/spec.toml
```

Edit `watch_paths` in the copied spec first. Starting daemon mode watches those configured locations.
The daemon reloads the spec after a file change or SIGHUP.

Run the checks:

```bash
cargo fmt --all -- --check
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
```

## Limits

- A score measures configured writing rules. It does not verify facts, completed work, or permission to act.
- Scoring alone does not block an outbound action. Enforce approval in the executing service.
- Quotations and task-specific language can trigger false matches. Review violations before changing rules.
- Transcript ingestion supports the implemented Claude and Codex message formats. Event-only Codex exports are not scored.
- Parser state stays in memory. A restart can replay existing records after the next file change.
- Use append-only transcripts. Same-inode rewrites can evade truncation detection if they regain their prior length before observation.
- The Unix metadata adapter supports macOS and Linux. This repository has no verified Windows runtime.

See [Build a macOS release](RELEASING.md) for packaging.

## Related repositories

- [agent-oversight](https://github.com/b2bvic/agent-oversight): orchestration cluster and evaluation guide.
- [skills](https://github.com/b2bvic/skills): quality threshold and local artifact checks.
- [observer-protocol](https://github.com/b2bvic/observer-protocol): Markdown intake and local review records.
- [session-ledger](https://github.com/b2bvic/session-ledger): searchable transcript archive.

## How this was built

This README was written with model assistance in 2026. The code and tests in this repository are the evidence; read them to judge the tool.

## License

[MIT](LICENSE).
