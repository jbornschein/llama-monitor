# llama-monitor

Terminal UI for monitoring a [llama.cpp](https://github.com/ggml-org/llama.cpp) router server.

![idle and active slots with prefill/generate states and t/s sparkline]

## Features

- Lists all models known to the router, showing which are loaded
- Per-slot state: **idle**, **prefill**, **generate**
- Generated token count and tokens/sec per slot
- Rolling t/s sparkline per model
- Auto-refreshes every second (configurable)

## Usage

```
llama-monitor [OPTIONS]
```

### CLI flags

| Flag | Env variable | Description | Default |
|------|-------------|-------------|---------|
| `--url <URL>` | `LLM_DEFAULT_URL` | Router server URL | `http://localhost:8080` |
| `--key <KEY>` | `LLM_DEFAULT_KEY` | API key for authentication | `KEY-SECRET` |
| `-i, --interval <SECS>` | — | Refresh interval in seconds | `1` |

CLI flags override environment variables.

### Key bindings

| Key | Action |
|-----|--------|
| `r` | Force refresh |
| `↑` / `↓` | Scroll |
| `q` / `Esc` | Quit |

## Installation

```
cargo install --git https://github.com/jbornschein/llama-monitor
```

Or clone, build and run locally:

```
git clone https://github.com/jbornschein/llama-monitor
cd llama-monitor
cargo run -- [OPTIONS]
```

Requires [Rust](https://rustup.rs/) 1.74 or later.

## Requirements

A llama.cpp server (`llama-server`) reachable at `http://localhost:8080`. The per-model `/slots` endpoint must be enabled (it is by default; disable with `--slots-endpoint-disable`).

## Notes

**Prefill detection** — llama.cpp does not expose slot phase in the `/slots` response (only a boolean `is_processing`). Prefill is inferred by tracking `id_task` transitions: a slot is considered in prefill from the moment a new task is assigned until the first token is decoded.

**Prompt token count** — not available via the `/slots` API. The field `n_prompt_tokens_processed` exists in the llama.cpp slot struct but is not included in the JSON response.
