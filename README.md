# Commitbot

A Rust-powered CLI that writes meaningful, structured Git commit messages using LLMs.

[![Version](https://img.shields.io/github/v/release/MikeGarde/commitbot?color=brightgreen&label=release)](https://github.com/MikeGarde/commitbot/releases)
[![Version](https://img.shields.io/badge/license-GPL--3.0-blue.svg)](https://github.com/MikeGarde/commitbot/blob/main/LICENSE)

---

**Commitbot** analyzes your staged Git changes and helps you craft clear, consistent commit messages that describe *why* changes were made — not just *what* changed.

It can summarize diffs, ask you how each file relates to the purpose of the commit, and produce structured, readable messages your teammates (and future self) will thank you for.

---

## Features

- **Interactive “ask” mode** – Classify each file as main, supporting, or consequential.
- **Quick mode** – Instantly summarize staged diffs into a commit message.
- **LLM-powered** – Uses OpenAI’s GPT models to generate concise and structured messages.
- **Configurable** – Choose models, tweak behavior, and set defaults in a config file.
- **Pull request summaries** – Generate clean, readable PR descriptions from your commit history.

---

## Installation

You’ll need an OpenAI API key set as an environment variable:

```bash
export OPENAI_API_KEY="sk-..."
```

### Homebrew

```bash
brew tap mikegarde/tap
brew install commitbot
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

Example:

```toml
model = "gpt-4o-mini"

["mikegarde/commitbot"]
model = "gpt-5-nano"
```

---

## Roadmap

- [x] Support for local/offline LLMs (Ollama, LM Studio).
- [ ] Model auto-detection and fallback.
- [ ] Configurable commit message templates.
- [ ] Integration with GitHub Actions or CI pipelines.

## License

**GPL-3.0-or-later**

See [LICENSE](./LICENSE) for details.

---

_Commitbot is under active development — features and output quality will evolve with each release._
