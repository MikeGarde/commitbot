# commitbot

**commitbot** is a Rust-based CLI tool that helps generate clear, structured Git commit messages using an LLM (such as OpenAI’s GPT models).  
It can analyze your staged changes, summarize each file interactively, and produce a well-organized commit message describing the intent behind the changes.

## Features

- Interactive "ask" mode to classify each file as main, supporting, or consequential.
- Simple one-shot mode for fast commits.
- Configurable model selection (e.g. `gpt-4o-mini`).

> **⚠️ Privacy Notice**  
> At this time, **commitbot** does not support using a local LLM model.  
> When the `--model` option is enabled, your staged diffs are sent to the configured API provider (e.g., OpenAI) for analysis.  
> Future versions will introduce support for specifying a custom API endpoint and integrating with self-hosted or alternative LLM providers to keep all processing local or at least internal.

## Installation

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
cargo install --git https://github.com/YOUR_USERNAME/commitbot --force
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

### OpenAI / ChatGPT

This needs your API Key as an environment variable.

```
export OPENAI_API_KEY="sk-..."
```

## License

GPL-3.0

---

*This project is in early development. Additional documentation and features will be added as the tool evolves.*
