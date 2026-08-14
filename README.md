# Commitbot

A Rust-powered CLI that writes meaningful, structured Git commit messages using LLMs.

[![Version](https://img.shields.io/github/v/release/MikeGarde/commitbot?color=blue&label=release)](https://github.com/MikeGarde/commitbot/releases)
[![License](https://img.shields.io/badge/license-GPL--3.0-blue.svg)](https://github.com/MikeGarde/commitbot/blob/main/LICENSE)
[![Downloads](https://img.shields.io/github/downloads/MikeGarde/commitbot/total.svg?color=blue)](https://GitHub.com/MikeGarde/commitbot/releases/)

---

**Commitbot** analyzes your staged Git changes and helps you craft clear, consistent commit messages that describe *why* changes were made — not just *what* changed.

It can summarize diffs, ask you how each file relates to the purpose of the commit, and produce structured, readable messages your teammates (and future self) will thank you for.

---

## Features

- **Interactive “ask” mode** – Classify each file as main, supporting, or consequential.
- **Quick mode** – Instantly summarize staged diffs into a commit message.
- **LLM-powered** – Uses OpenAI, or a local model via [Ollama](#providers) or [LM Studio](#providers).
- **Configurable** – Choose models, tweak behavior, and set defaults in a config file.
- **Pull request summaries** – Generate clean, readable PR descriptions from your commit history.

---

## Installation

To use OpenAI (the default provider) you’ll need an API key set as an environment variable:

```bash
export OPENAI_API_KEY="sk-..."
```

### Homebrew

```bash
brew install mikegarde/tap/commitbot
```

---

## Usage

### Simple Mode

Analyze all staged changes and generate a commit message in one step:

```bash
commitbot
```

---

### Interactive Mode

Walk through each staged file and describe how it relates to the main purpose of the commit:

```bash
commitbot --ask
```

For each file, select:

```
1) Main purpose
2) Supporting change
3) Consequence / ripple
4) Ignore
```

After all files are classified, Commitbot summarizes and generates the full commit message.

---

### Using External Diffs

Generate commit messages from a saved diff file instead of git staged changes:

```bash
# From a diff file
commitbot --diff my-changes.diff

# With a custom branch name for context
commitbot --diff my-changes.diff --branch feature/ISSUE-123-auth

# From stdin (pipe a diff)
git diff HEAD~3 | commitbot --diff -
```

---

### Pull Request Summaries

Generate high-level PR descriptions by summarizing commit messages instead of diffs:

```bash
commitbot pr develop
commitbot pr develop feat/ISSUE-201-registration
```

Commitbot will:

- Collect all commits between the base (`develop` or `main`) and the feature branch.
- Group commits referencing PR numbers (e.g. `#123`).
- Summarize them into a clear, cohesive description.

---

## Configuration

Commitbot automatically loads its configuration from [~/.config/commitbot.toml](./commitbot.toml).
Settings can be defined globally in this file, overridden by environment variables, or specified directly through CLI flags.
Per-project configurations are also supported for repository-specific overrides.

> Note: Repository names are case-sensitive.

Example:

```toml
model = "gpt-4o-mini"

["MikeGarde/commitbot"]
model = "gpt-5-nano"
```

---

## Providers

Set `provider` to choose a backend. Any of them can be overridden per repository.

| Provider   | Default `url`            | API key  |
|------------|--------------------------|----------|
| `openai`   | `https://api.openai.com` | required |
| `ollama`   | `http://localhost:11434` | not used |
| `lmstudio` | `http://localhost:1234`  | optional |

```toml
[default]
provider = "lmstudio"
model = "qwen/qwen3-coder-30b"
url = "http://192.168.1.16:1234/v1"
```

---

## Roadmap

- [x] Support for local/offline LLMs (Ollama, LM Studio).
- [ ] Support for Anthropic's Claude.
- [ ] Model auto-detection and fallback.
- [ ] Configurable commit message templates.
- [ ] Integration with GitHub Actions or CI pipelines.

## License

**GPL-3.0-or-later**

See [LICENSE](./LICENSE) for details.

---

_Commitbot is under active development — features and output quality will evolve with each release._
