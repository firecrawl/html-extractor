# html-extractor

A fast, streaming, page-type-aware HTML main-content extractor in Rust, with NAPI bindings for Node.js. A general-purpose library for pulling article-style content out of raw HTML pages.

Algorithm inspired by Python's [trafilatura](https://github.com/adbar/trafilatura) (`~/Code/trafilatura` is the read-only reference). Implementation is from scratch — **do not port from any existing Rust/Go/JS port**. Inspiration is from the algorithm + decisions documented in the Python source; the implementation, API surface, optimizations, and architecture are ours.

## What this is

A library you call with raw HTML and get back the article content, stripped of nav/footer/related-stories/ads/site chrome. Output is markdown by default (clean text with headings, lists, tables, code blocks preserved). Also returns a per-extraction confidence score, the detected page type, and metadata.

## Why it exists

- Heuristic CSS-selector blocklists are brittle: they break on hashed class names and don't generalize across non-article page types (product, listing, forum, documentation).
- Existing Rust ports of trafilatura are pre-1.0 with limited maintenance.
- A self-contained library we can ship as open source and progressively optimize.

## Documents

Read these in order if you're new (or if you're the agent driving the implementation loop):

1. **[SPEC.md](./SPEC.md)** — what to build. Input/output contract, API surface, configuration options, error handling, performance targets, acceptance criteria.
2. **[ALGORITHM.md](./ALGORITHM.md)** — how the trafilatura-inspired algorithm works. Five-stage pipeline, per-feature scoring, fallback chain. Includes pointers to the relevant files in `~/Code/trafilatura` so you can study the reference implementation.
3. **[OPTIMIZATIONS.md](./OPTIMIZATIONS.md)** — what we want to do differently from trafilatura, and why. Streaming SAX parser, page-type-aware extraction, per-element confidence, computed-style hints, native interop with `simd-html-to-md`.
4. **[TESTING.md](./TESTING.md)** — test strategy. Golden corpus, fixtures, unit + integration + benchmark requirements. CI gates.
5. **[ROADMAP.md](./ROADMAP.md)** — phased delivery. Each phase has acceptance criteria and ships independent value.
6. **[CLAUDE.md](./CLAUDE.md)** — agent-specific operating instructions. Constraints (do not push, do not consult ports, etc.), expected commit cadence, what to do when stuck.

## High-level architecture

```
        ┌──────────────────────────────────────────────┐
        │  HTML bytes in                                │
        └──────────────────────────────────────────────┘
                          │
                          ▼
        ┌──────────────────────────────────────────────┐
        │  Stage 1 — pre-clean                          │
        │  drop <script>/<style>/<head>/comments        │
        └──────────────────────────────────────────────┘
                          │
                          ▼
        ┌──────────────────────────────────────────────┐
        │  Stage 2 — page-type classification            │
        │  pick a scoring profile per type              │
        └──────────────────────────────────────────────┘
                          │
                          ▼
        ┌──────────────────────────────────────────────┐
        │  Stage 3 — score + select main subtree         │
        │  text density / link density / tag weights /   │
        │  class hints / position / parent chain         │
        └──────────────────────────────────────────────┘
                          │
                          ▼
        ┌──────────────────────────────────────────────┐
        │  Stage 4 — fallback chain if Stage 3 degraded  │
        │  justext-style, readability-style, raw text    │
        └──────────────────────────────────────────────┘
                          │
                          ▼
        ┌──────────────────────────────────────────────┐
        │  Stage 5 — post-clean + markdown render        │
        └──────────────────────────────────────────────┘
                          │
                          ▼
        ┌──────────────────────────────────────────────┐
        │  ExtractResult { markdown, page_type,         │
        │                   extraction_quality, ...}    │
        └──────────────────────────────────────────────┘
```

## Tech stack (high level)

- **Rust** for the core library. Modern, idiomatic, no `unsafe` unless justified.
- **NAPI bindings** via `napi-rs` for Node.js / Bun consumers. Pre-built binaries for the platforms Firecrawl deploys to.
- **`criterion`** for benchmarks. Throughput numbers in CI.
- **Golden corpus** of HTML fixtures with expected extractions, in the test suite.

Crate name is TBD — the working name in code can be whatever; final cargo/npm name will be decided before first publish. Suggested options include `marrow`, `crux`, `pith`, `vellum`. Use a placeholder in `Cargo.toml` until the call is made.

## Status

Phase 1 (algorithm parity) and Phase 2 (NAPI bindings) are complete. See `ROADMAP.md` for the per-checkbox state and `STATUS.md` for the summary.

- 34 Rust unit + integration + doctests
- 54 golden-corpus fixtures across 8 categories, all passing
- 7 NAPI binding tests, all passing

## Use from Rust

```toml
[dependencies]
html-extractor = "0.1"
```

```rust
use html_extractor::{extract, ExtractOptions};

let html = std::fs::read_to_string("page.html")?;
let result = extract(&html, &ExtractOptions::default())?;
println!("{}", result.markdown);
println!("page_type = {:?}", result.page_type);
println!("quality   = {:.2}", result.extraction_quality);
```

Run the bundled example: `cargo run --example extract_one -p html-extractor`.

## Use from Node

```bash
cd crates/html-extractor-napi
npm install
npm run build           # produces html-extractor.<triple>.node for this host
```

```js
import { extract } from 'html-extractor'        // or use require()

const html = '<html>…</html>'
const result = await extract(html, { url: 'https://example.com/article' })
console.log(result.markdown)
console.log(result.metadata)
```

Run the bundled example: `node examples/node-extract.mjs`.

## Throughput

`cargo bench -p html-extractor --bench throughput -- --quick` on an Apple M-series, release build:

| Input               | Time    | Throughput   |
|---------------------|---------|--------------|
| Small (~10 KB)      | ~29 µs  | ~67 MiB/s    |
| Medium (~148 KB)    | ~900 µs | ~156 MiB/s   |
| Large (~1.7 MB)     | ~10 ms  | ~157 MiB/s   |

These are DOM-based numbers. A streaming backend (planned) is expected to lift these further.

## License

TBD — most likely Apache-2.0 (matches Firecrawl's other OSS).
