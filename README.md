# observer-daemon

You can miss repeated writing-rule violations when you review agent responses by hand. Use observer-daemon to score transcripts and record failures outside the model.

Install with Rust and Cargo:

```bash
cargo install --git https://github.com/b2bvic/observer-daemon.git --locked
```

Sample output from `observer-daemon --config spec.toml --validate "The file is ready."` when no rules fail:

```text
Class: generic
Score: 100/100
No violations.
```

Your rules determine the score. Configure `spec.toml` before you run the command. Transcript scoring alone does not block an outbound action.

## What it does

- **Watches** transcript directories (`watch_paths` in `spec.toml`) with debounce, hot-reloading its spec on file change or SIGHUP.
- **Classifies** each prompt into a content class before scoring: `legal`, `career_application`, `content_copy`, `technical_build`, `conversational`, or `generic` fallback. Multi-signal prompts resolve by fixed priority. A career-strategy briefing that mentions a compliance role is career material, and the test suite pins that exact case as a named regression.
- **Scores** the response with the rubric for its class: a stack of deterministic validators (sycophancy, filler, crutch words, punctuation habits, lexical density, rhythm, model tells, policy-pack literals) each deducting from 100. Below `passing_score`, the response fails.
- **Records** every validation to a JSONL ledger, and every correction to a corrections ledger. Corrections are class-scoped: a rule promoted from legal failures does not silently constrain conversational writing. Records from before class scoping remain valid as `generic`.

## Why a daemon and not a prompt

Prompts drift and get truncated; a standard enforced outside the model's context cannot be talked out of. The daemon is the mechanical half of a two-part contract: the model generates, the daemon verifies against the operator's recorded taste. Failures compound into the standard instead of evaporating.

## Worked example

Feed it a transcript directory and a spec:

```
cargo run -- --daemon --config spec.toml
```

Write a response containing "I hope this helps! Let me know if you'd like me to elaborate" into a watched path. The validation ledger records the sycophancy and filler deductions with the exact phrases, the score, and the content class that selected the rubric. Correct it, and the correction ledger holds the before and after as a durable example.

## Configuration

`spec.toml.example` documents the full surface: watch paths, debounce, ledger locations, scoring thresholds, and the policy pack (blocked literals and regexes with per-issue deductions). Copy it to `spec.toml` and point the paths at your own transcript locations.

## How this was built

Specification, taxonomy, priorities, and the behavioral standard: human judgment, mine. Implementation: AI models executing that specification under a build contract with an adversarial audit before publish. The correction-ledger design ships here; my personal correction content does not. All fixtures are synthetic.

## Principles

Part of a larger system: this repository proves **P11 (voice is a written standard)** and **P17 (the system learns through correction)** from the [Seventeen Principles](https://victorvalentineromo.com/principles). The written standard itself is documented in [observer-protocol](https://github.com/b2bvic/observer-protocol).
