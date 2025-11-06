# Commit Bot

**commitbot** is a Rust-based CLI tool that helps generate clear, structured Git commit messages using an LLM (such as OpenAI’s GPT models).  

It can analyze your staged changes, summarize each file interactively, and produce a well-organized commit message describing the intent behind the changes.

## Features

- Interactive "ask" mode to classify each file as main, supporting, or consequential.
- Simple one-shot mode for fast commits.
- Configurable model selection (e.g. `gpt-4o-mini`).

### ⚠️ Privacy Notice
At this time, `commitbot` does not support using a local LLM model.
Your staged diffs are sent to OpenAI for analysis. 

Future versions will introduce support for specifying a custom API endpoint and integrating with self-hosted or 
alternative LLM providers to keep all processing local or at least internal.

## Prove It!

All [commit](https://github.com/MikeGarde/commitbot/commits/) & 
[PR](https://github.com/MikeGarde/commitbot/pulls?q=is%3Apr+is%3Aclosed) 
messages in this repo will be generated using `commitbot`. Although we should all eat our own dog food, we still 
recommend the smell test before committing here or anywhere else!

## Installation

### Prerequisites

`commitbot` needs your OpenAI API Key as an environment variable.

```
export OPENAI_API_KEY="sk-..."
```

### Easy - Coming Soon

Placeholder for downloading pre-built binaries.

### From source (local development)

```
git clone https://github.com/mikegarde/commitbot.git
cd commitbot
cargo install --path . --force
```

This installs the binary into `~/.cargo/bin/commitbot`.

Make sure `~/.cargo/bin` is in your PATH:

```
export PATH="$HOME/.cargo/bin:$PATH"
```

### From Git (no manual clone)

```
cargo install --git https://github.com/mikegarde/commitbot --force
```

## Usage

### Simple mode

Analyze all staged changes and generate a commit message in one step:

```
commitbot
```

### Interactive "ask" mode

Walks through each staged file and asks how it relates to the main purpose of the commit:

```
commitbot --ask
```

For each file you can choose:

```
1) Main purpose
2) Supporting change
3) Consequence / ripple
4) Ignore
```

After all files are classified, **commitbot** summarizes and generates a full commit message.

### Pull Request Summaries

`commitbot` can also generate clear, high-level **Pull Request descriptions** by summarizing the commit history between two branches.  
Instead of sending an enormous diff to the model, it analyzes the **commit or PR messages** to produce a concise overview of the feature branch’s purpose and major changes.

- It collects all commits between a **base** branch (such as `develop` or `main`) and the **feature** branch.
- If multiple PR numbers are detected in commit messages (e.g., `#123`), `commitbot` groups them and references each PR in the summary.
- Otherwise, it summarizes the commits directly.
- The tool can also be forced into either mode with flags.

```
commitbot pr develop
commitbot pr develop feat/ISSUE-201-registration
```

## License

GPL-3.0

---

*This project is in early development. Additional documentation and features will be added as the tool evolves.*
