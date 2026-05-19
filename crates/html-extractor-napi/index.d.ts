/* eslint-disable */
/// Type declarations for `html-extractor`.

export interface ExtractOptions {
  /** Original URL of the page (for relative→absolute link rewriting and page-type classification). */
  url?: string
  /** Be more aggressive about dropping anything ambiguous. */
  favorPrecision?: boolean
  /** Be more lenient — keep ambiguous content. Mutually exclusive with favorPrecision. */
  favorRecall?: boolean
  /** Include a plain-text mirror of the markdown in the result. */
  outputText?: boolean
  /** Reserved for Phase 4 (per-element decisions ledger). */
  outputDecisions?: boolean
  /** Hint for language detection. */
  targetLanguage?: string
  /** Skip the classifier and use this page type's scoring profile directly. */
  pageTypeOverride?:
    | 'article'
    | 'forum'
    | 'product'
    | 'listing'
    | 'collection'
    | 'documentation'
    | 'service'
    | 'other'
  /** Preserve `<a>` tags as `[text](href)` in markdown. Default true. */
  includeLinks?: boolean
  /** Preserve tables as GFM tables in markdown. Default true. */
  includeTables?: boolean
  /** Preserve `<img>` as `![alt](src)` in markdown. Default false. */
  includeImages?: boolean
  /** Populate the `metadata` field on the result. Default true. */
  includeMetadata?: boolean
  /** If the kept subtree's text is shorter than this many characters, fall back. Default 25. */
  minExtractionLength?: number
}

export interface Metadata {
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

export interface ExtractStats {
  textChars: number
  elementCount: number
  usedFallback: boolean
  pageType: string
}

export interface ExtractResult {
  /** Cleaned main content as GitHub-flavored markdown. */
  markdown: string
  /** Plain-text variant when `outputText: true`. */
  text?: string
  /** Detected page type. */
  pageType: string
  /** Confidence in `[0.0, 1.0]`. */
  extractionQuality: number
  /** BCP-47 language tag if detectable. */
  language?: string
  /** Metadata harvested from JSON-LD, OpenGraph, etc. */
  metadata?: Metadata
  /** Internal stats useful for telemetry. */
  stats?: ExtractStats
  /** Reason for a low-confidence / failed extraction, if any. */
  errorReason?: string
}

/** Extract the main content asynchronously (CPU work runs on a worker thread). */
export function extract(html: string, options?: ExtractOptions): Promise<ExtractResult>

/** Synchronous variant. Blocks the event loop while running; prefer `extract` for large pages. */
export function extractSync(html: string, options?: ExtractOptions): ExtractResult

/** Library version. */
export function version(): string
