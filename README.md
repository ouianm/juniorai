# JuniorAI

JuniorAI is a local-first debugging process analyzer written in Rust.

It records development sessions, captures executed commands and Git context,
and generates a debugging autopsy based on the observed workflow.

## Current Features

- Start and stop debugging sessions
- Record commands executed through JuniorAI
- Capture command output, errors and exit codes
- Capture the active Git branch
- Capture changed files before and after commands
- Detect commands that changed the working tree
- Detect failed commands
- Detect repeated commands
- Generate a local debugging autopsy
- Store all session data locally as JSON
- Express uncertain conclusions with confidence and alternative explanations

## Example

```bash
cargo run -- start --goal "Understand a failing test"
cargo run -- run "cargo test"
cargo run -- run "git status"
cargo run -- stop
cargo run -- autopsy
```

## Current Limitations

JuniorAI does not yet detect deeper debugging patterns such as:

- Modifying code before reproducing an error
- Changing code without testing afterwards
- Repeatedly trying the same unsuccessful approach
- Making too many changes at once
- Long-term personal debugging patterns

A VS Code extension, dashboard and LLM-based analysis are also not included yet.

## Vision

JuniorAI aims to become a local-first debugging process intelligence tool.

Instead of automatically fixing bugs, it analyzes how developers approach
problems and helps them improve their debugging methodology.
