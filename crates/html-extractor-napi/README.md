# html-extractor

Fast HTML main-content extractor for Node.js / Bun. Native Rust performance via [napi-rs](https://napi.rs).

Pulls the article body out of a raw HTML page and renders it as clean markdown — stripping nav, footers, related-stories rails, ads, and other site chrome. Page-type aware (article, forum, product, listing, documentation, etc.), with a per-extraction confidence score and harvested metadata (JSON-LD, OpenGraph, microformats).

Built by [Firecrawl](https://firecrawl.dev). Source: [github.com/firecrawl/html-extractor](https://github.com/firecrawl/html-extractor).

## Install

```bash
npm install @firecrawl/html-extractor
# or
bun add @firecrawl/html-extractor
```

Prebuilt binaries included for **linux-x64**, **macOS ARM64**, and **windows-x64**. No Rust toolchain needed.

## Quick start

```typescript
import { extract } from '@firecrawl/html-extractor'

const html = await fetch('https://example.com/article').then(r => r.text())
const result = await extract(html, { url: 'https://example.com/article' })

console.log(result.markdown)           // cleaned article as markdown
console.log(result.pageType)           // 'article' | 'forum' | 'product' | ...
console.log(result.extractionQuality)  // 0.0..1.0 confidence
console.log(result.metadata?.title)    // 'How to do the thing'
```

## API

### `extract(html, options?): Promise<ExtractResult>`

Asynchronous extraction. CPU work runs on a libuv worker thread, so the Node event loop isn't blocked for large pages. **Prefer this for production use.**

### `extractSync(html, options?): ExtractResult`

Synchronous variant. Convenient for scripts and short HTML, but blocks the event loop while running. Don't use in request handlers.

### `version(): string`

Returns the library version.

## Options

```typescript
interface ExtractOptions {
  url?: string                    // original URL, used for absolute-link rewriting + classification
  favorPrecision?: boolean        // be aggressive about dropping ambiguous content
  favorRecall?: boolean           // be lenient — keep ambiguous content (mutually exclusive with favorPrecision)
  outputText?: boolean            // also return a plain-text mirror of the markdown
  targetLanguage?: string         // hint for language detection
  pageTypeOverride?:              // skip the classifier and force a scoring profile
    | 'article'   | 'forum'
    | 'product'   | 'listing'
    | 'collection'| 'documentation'
    | 'service'   | 'other'
  includeLinks?: boolean          // preserve <a> as [text](href) in markdown (default true)
  includeTables?: boolean         // preserve tables as GFM tables (default true)
  includeImages?: boolean         // preserve <img> as ![alt](src) (default false)
  includeMetadata?: boolean       // populate the metadata field (default true)
  minExtractionLength?: number    // fall back if the kept subtree is shorter than this (default 25)
}
```

## Result shape

```typescript
interface ExtractResult {
  markdown: string                // cleaned main content as GFM markdown
  text?: string                   // plain-text mirror, present when outputText: true
  pageType: string                // detected page type
  extractionQuality: number       // confidence in [0.0, 1.0]
  language?: string               // BCP-47 language tag
  metadata?: {
    title?: string
    description?: string
    author?: string
    publishedDate?: string
    siteName?: string
    imageUrl?: string
    canonicalUrl?: string
    language?: string
    keywords: string[]
  }
  stats?: {
    textChars: number
    elementCount: number
    usedFallback: boolean
    pageType: string
  }
  errorReason?: string            // present on low-confidence extractions
}
```

## How the algorithm works

A trafilatura-inspired five-stage pipeline:

1. **Pre-clean** — drop `<script>` / `<style>` / `<head>` / comments / hidden elements.
2. **Page-type classification** — rules-based ladder (URL patterns, tag counts, class regexes, JSON-LD `@type`) picks a scoring profile.
3. **Score + select** — every element scored on 7 features (text density, link density, tag weight, class hints, position, parent chain); the highest-scoring subtree is the candidate main content.
4. **Fallback chain** — if Stage 3 produced a degenerate result, fall through justext-style → readability-style → raw-text.
5. **Post-clean + render** — strip leftover boilerplate from inside the kept subtree, render markdown, extract metadata.

See the [Rust crate's README](https://github.com/firecrawl/html-extractor) for architecture details and benchmarks.

## License

[Apache-2.0](https://github.com/firecrawl/html-extractor/blob/main/LICENSE).
