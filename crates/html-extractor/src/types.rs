//! Public types: options, results, errors, metadata, page-type.

use std::fmt;

/// The page-type label produced by the Stage-2 classifier.
///
/// Mirrors the seven types documented in `ALGORITHM.md` plus `Other` as the
/// fallback when no signal is confident.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum PageType {
    /// News, blog, long-form prose.
    Article,
    /// Threaded discussion (HN, Reddit, Stack Exchange).
    Forum,
    /// E-commerce product detail page.
    Product,
    /// Search results, product grids, news indexes.
    Listing,
    /// Homepage-style aggregator.
    Collection,
    /// Technical docs with TOC + body.
    Documentation,
    /// Marketing, pricing, contact, terms-of-service.
    Service,
    /// Catch-all when nothing else fits.
    Other,
}

impl PageType {
    /// Lowercase string form, suitable for JSON / NAPI output.
    pub fn as_str(&self) -> &'static str {
        match self {
            PageType::Article => "article",
            PageType::Forum => "forum",
            PageType::Product => "product",
            PageType::Listing => "listing",
            PageType::Collection => "collection",
            PageType::Documentation => "documentation",
            PageType::Service => "service",
            PageType::Other => "other",
        }
    }
}

impl fmt::Display for PageType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Caller-provided options. All fields optional with sensible defaults.
#[derive(Debug, Clone)]
pub struct ExtractOptions {
    /// Original URL of the page. Used for relative→absolute link rewriting,
    /// page-type classification, and language fallback.
    pub url: Option<String>,
    /// Be more aggressive about dropping anything ambiguous. Lower recall,
    /// higher precision. Mutually exclusive with [`favor_recall`].
    pub favor_precision: bool,
    /// Be more lenient — keep ambiguous content. Higher recall, lower
    /// precision. Mutually exclusive with [`favor_precision`].
    pub favor_recall: bool,
    /// Include the plain-text version in the result.
    pub output_text: bool,
    /// Include the per-element decisions ledger (currently always `None`; see
    /// `DECISIONS.md` D10).
    pub output_decisions: bool,
    /// Hint for language detection.
    pub target_language: Option<String>,
    /// Skip the classifier and use this page type directly.
    pub page_type_override: Option<PageType>,
    /// Preserve `<a>` tags as `[text](href)` in markdown.
    pub include_links: bool,
    /// Preserve tables as GFM tables in markdown.
    pub include_tables: bool,
    /// Preserve `<img>` as `![alt](src)` in markdown.
    pub include_images: bool,
    /// Populate the `metadata` field.
    pub include_metadata: bool,
    /// If the kept subtree's text is shorter than this, fall back.
    pub min_extraction_length: usize,
}

impl Default for ExtractOptions {
    fn default() -> Self {
        Self {
            url: None,
            favor_precision: false,
            favor_recall: false,
            output_text: false,
            output_decisions: false,
            target_language: None,
            page_type_override: None,
            include_links: true,
            include_tables: true,
            include_images: false,
            include_metadata: true,
            min_extraction_length: 25,
        }
    }
}

/// The full extraction result returned by [`crate::extract`].
#[derive(Debug, Clone)]
pub struct ExtractResult {
    /// Cleaned main content, GFM markdown.
    pub markdown: String,
    /// Plain-text variant, present only when `options.output_text` is true.
    pub text: Option<String>,
    /// Detected page type.
    pub page_type: PageType,
    /// Confidence in the extraction, in `[0.0, 1.0]`.
    pub extraction_quality: f32,
    /// BCP-47 language tag if detectable.
    pub language: Option<String>,
    /// Metadata pulled from JSON-LD, OpenGraph, etc.
    pub metadata: Option<Metadata>,
    /// Per-element ledger (Phase 4; presently always `None`).
    pub decisions: Option<Vec<Decision>>,
    /// Stats describing what happened internally.
    pub stats: Option<ExtractStats>,
    /// If extraction failed gracefully, the structured reason.
    pub error_reason: Option<ExtractError>,
}

impl ExtractResult {
    pub(crate) fn empty(reason: ExtractError) -> Self {
        Self {
            markdown: String::new(),
            text: None,
            page_type: PageType::Other,
            extraction_quality: 0.0,
            language: None,
            metadata: None,
            decisions: None,
            stats: None,
            error_reason: Some(reason),
        }
    }
}

/// Structured metadata harvested from JSON-LD / OG / `<meta>` / `<title>`.
#[derive(Debug, Clone, Default)]
pub struct Metadata {
    /// Page title.
    pub title: Option<String>,
    /// Page description / summary.
    pub description: Option<String>,
    /// Author byline.
    pub author: Option<String>,
    /// Publication date (ISO 8601 if parseable).
    pub published_date: Option<String>,
    /// Site name (`og:site_name`, etc.).
    pub site_name: Option<String>,
    /// Hero image URL.
    pub image_url: Option<String>,
    /// Canonical URL (from `<link rel="canonical">`).
    pub canonical_url: Option<String>,
    /// Detected language (`<html lang="">` or meta).
    pub language: Option<String>,
    /// Comma-separated keywords / tags.
    pub keywords: Vec<String>,
}

/// Per-element decision (Phase 4 stub).
#[derive(Debug, Clone)]
pub struct Decision {
    /// Selector identifying the element.
    pub selector: String,
    /// Raw score.
    pub score: f32,
    /// `true` if the element was kept in the output.
    pub kept: bool,
    /// Confidence in `[0.0, 1.0]`.
    pub confidence: f32,
}

/// Internal stats useful for telemetry.
#[derive(Debug, Clone)]
pub struct ExtractStats {
    /// Number of UTF-8 characters in the extracted markdown's plain text form.
    pub text_chars: usize,
    /// Number of internal-tree elements walked.
    pub element_count: usize,
    /// True if a Stage-4 fallback was used.
    pub used_fallback: bool,
    /// The page type used for scoring.
    pub page_type: PageType,
}

/// Structured error returned (or returned-via-`error_reason`) by the extractor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExtractError {
    /// Empty or whitespace-only input.
    EmptyInput,
    /// The HTML parser failed catastrophically.
    ParseFailure(&'static str),
    /// Options that can't both be true were both set.
    ConflictingOptions(&'static str),
}

impl fmt::Display for ExtractError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ExtractError::EmptyInput => f.write_str("empty input"),
            ExtractError::ParseFailure(msg) => write!(f, "parse failure: {msg}"),
            ExtractError::ConflictingOptions(msg) => write!(f, "conflicting options: {msg}"),
        }
    }
}

impl std::error::Error for ExtractError {}
