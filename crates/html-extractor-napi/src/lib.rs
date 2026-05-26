//! NAPI bindings for `html-extractor`.
//!
//! Single exported function `extract(html, options?) -> Promise<ExtractResult>`.
//! Heavy CPU work is moved to a worker thread via napi-rs `Task` so the Node
//! event loop stays free; a `extractSync` mirror is also provided for callers
//! that want to skip the Promise hop.

#![deny(clippy::all)]

use napi::bindgen_prelude::*;
use napi_derive::napi;

#[napi(object)]
#[derive(Default)]
pub struct ExtractOptions {
    pub url: Option<String>,
    pub favor_precision: Option<bool>,
    pub favor_recall: Option<bool>,
    pub output_text: Option<bool>,
    pub output_decisions: Option<bool>,
    pub target_language: Option<String>,
    pub page_type_override: Option<String>,
    pub include_links: Option<bool>,
    pub include_tables: Option<bool>,
    pub include_images: Option<bool>,
    pub include_metadata: Option<bool>,
    pub min_extraction_length: Option<u32>,
    /// Reject inputs larger than this many bytes; returns an empty result with
    /// `errorReason = "input_too_large"`. Default: no limit. Recommended for
    /// production: cap at the 99.9th-percentile page size on your traffic.
    pub max_input_size: Option<u32>,
}

#[napi(object)]
pub struct Metadata {
    pub title: Option<String>,
    pub description: Option<String>,
    pub author: Option<String>,
    pub published_date: Option<String>,
    pub site_name: Option<String>,
    pub image_url: Option<String>,
    pub canonical_url: Option<String>,
    pub language: Option<String>,
    pub keywords: Vec<String>,
}

#[napi(object)]
pub struct ExtractStats {
    pub text_chars: u32,
    pub element_count: u32,
    pub used_fallback: bool,
    pub page_type: String,
}

#[napi(object)]
pub struct ExtractResult {
    pub markdown: String,
    pub text: Option<String>,
    pub page_type: String,
    pub extraction_quality: f64,
    pub language: Option<String>,
    pub metadata: Option<Metadata>,
    pub stats: Option<ExtractStats>,
    pub error_reason: Option<String>,
}

fn map_options(o: &ExtractOptions) -> html_extractor::ExtractOptions {
    let mut opts = html_extractor::ExtractOptions::default();
    if let Some(u) = &o.url {
        opts.url = Some(u.clone());
    }
    if let Some(v) = o.favor_precision {
        opts.favor_precision = v;
    }
    if let Some(v) = o.favor_recall {
        opts.favor_recall = v;
    }
    if let Some(v) = o.output_text {
        opts.output_text = v;
    }
    if let Some(v) = o.output_decisions {
        opts.output_decisions = v;
    }
    if let Some(v) = &o.target_language {
        opts.target_language = Some(v.clone());
    }
    if let Some(v) = &o.page_type_override {
        opts.page_type_override = Some(parse_page_type(v));
    }
    if let Some(v) = o.include_links {
        opts.include_links = v;
    }
    if let Some(v) = o.include_tables {
        opts.include_tables = v;
    }
    if let Some(v) = o.include_images {
        opts.include_images = v;
    }
    if let Some(v) = o.include_metadata {
        opts.include_metadata = v;
    }
    if let Some(v) = o.min_extraction_length {
        opts.min_extraction_length = v as usize;
    }
    opts
}

fn parse_page_type(s: &str) -> html_extractor::PageType {
    match s.to_ascii_lowercase().as_str() {
        "article" => html_extractor::PageType::Article,
        "forum" => html_extractor::PageType::Forum,
        "product" => html_extractor::PageType::Product,
        "listing" => html_extractor::PageType::Listing,
        "collection" => html_extractor::PageType::Collection,
        "documentation" => html_extractor::PageType::Documentation,
        "service" => html_extractor::PageType::Service,
        _ => html_extractor::PageType::Other,
    }
}

fn map_result(r: html_extractor::ExtractResult, want_text: bool) -> ExtractResult {
    let text = if want_text {
        Some(strip_markdown(&r.markdown))
    } else {
        r.text
    };
    let metadata = r.metadata.map(|m| Metadata {
        title: m.title,
        description: m.description,
        author: m.author,
        published_date: m.published_date,
        site_name: m.site_name,
        image_url: m.image_url,
        canonical_url: m.canonical_url,
        language: m.language,
        keywords: m.keywords,
    });
    let stats = r.stats.map(|s| ExtractStats {
        text_chars: s.text_chars as u32,
        element_count: s.element_count as u32,
        used_fallback: s.used_fallback,
        page_type: s.page_type.to_string(),
    });
    ExtractResult {
        markdown: r.markdown,
        text,
        page_type: r.page_type.to_string(),
        extraction_quality: r.extraction_quality as f64,
        language: r.language,
        metadata,
        stats,
        error_reason: r.error_reason.map(|e| e.to_string()),
    }
}

/// Cheap markdown→plain-text rewrite: drop common markdown punctuation. Good
/// enough for tokenizers; not a full GFM parser.
fn strip_markdown(md: &str) -> String {
    let mut out = String::with_capacity(md.len());
    for line in md.lines() {
        let mut s = line.trim_start();
        s = s.trim_start_matches('#').trim_start();
        s = s.trim_start_matches("> ").trim_start_matches('>');
        // strip list markers
        if let Some(rest) = s.strip_prefix("- ") {
            s = rest;
        } else if let Some(rest) = s.split_once(". ") {
            if rest.0.chars().all(|c| c.is_ascii_digit()) {
                s = rest.1;
            }
        }
        // strip code fences
        if s.starts_with("```") {
            continue;
        }
        let mut chars = s.chars().peekable();
        while let Some(c) = chars.next() {
            match c {
                '*' | '_' | '`' | '~' => {}
                '[' => {
                    let mut depth = 1;
                    let mut text = String::new();
                    for cc in chars.by_ref() {
                        match cc {
                            '[' => depth += 1,
                            ']' => {
                                depth -= 1;
                                if depth == 0 {
                                    break;
                                }
                                text.push(cc);
                            }
                            _ => text.push(cc),
                        }
                    }
                    if let Some(&'(') = chars.peek() {
                        chars.next();
                        for cc in chars.by_ref() {
                            if cc == ')' {
                                break;
                            }
                        }
                    }
                    out.push_str(&text);
                }
                _ => out.push(c),
            }
        }
        out.push('\n');
    }
    out.trim().to_string()
}

/// Returns a "too large" empty result if `max_input_size` is set and exceeded.
/// Returning a result (rather than throwing) keeps the JS contract consistent
/// with how `html_extractor::extract` already reports empty / unparseable input.
fn too_large_result(html_len: usize, limit: usize) -> ExtractResult {
    ExtractResult {
        markdown: String::new(),
        text: None,
        page_type: "other".to_string(),
        extraction_quality: 0.0,
        language: None,
        metadata: None,
        stats: None,
        error_reason: Some(format!(
            "input_too_large: {html_len} bytes exceeds max_input_size {limit}"
        )),
    }
}

/// Synchronous variant. Prefer `extract` (async) for large inputs.
#[napi(js_name = "extractSync")]
pub fn extract_sync(html: String, options: Option<ExtractOptions>) -> Result<ExtractResult> {
    let opts_in = options.unwrap_or_default();
    if let Some(limit) = opts_in.max_input_size {
        let limit = limit as usize;
        if html.len() > limit {
            return Ok(too_large_result(html.len(), limit));
        }
    }
    let want_text = opts_in.output_text.unwrap_or(false);
    let opts = map_options(&opts_in);
    match html_extractor::extract(&html, &opts) {
        Ok(r) => Ok(map_result(r, want_text)),
        Err(e) => Err(Error::from_reason(e.to_string())),
    }
}

pub struct ExtractTask {
    pub(crate) html: String,
    pub(crate) opts: html_extractor::ExtractOptions,
    pub(crate) want_text: bool,
    /// Pre-resolved failure result (e.g. input exceeded max_input_size).
    /// When set, `compute` short-circuits and `resolve` returns this.
    pub(crate) preflight_failure: Option<ExtractResult>,
}

#[napi]
impl Task for ExtractTask {
    type Output = Option<html_extractor::ExtractResult>;
    type JsValue = ExtractResult;

    fn compute(&mut self) -> Result<Self::Output> {
        if self.preflight_failure.is_some() {
            return Ok(None);
        }
        html_extractor::extract(&self.html, &self.opts)
            .map(Some)
            .map_err(|e| Error::from_reason(e.to_string()))
    }

    fn resolve(&mut self, _env: Env, output: Self::Output) -> Result<Self::JsValue> {
        if let Some(fail) = self.preflight_failure.take() {
            return Ok(fail);
        }
        let r = output.expect("compute returned None without a preflight_failure set");
        Ok(map_result(r, self.want_text))
    }
}

/// Asynchronous extract. The CPU work runs on a libuv worker thread so the
/// Node event loop is not blocked.
#[napi(js_name = "extract")]
pub fn extract_async(html: String, options: Option<ExtractOptions>) -> AsyncTask<ExtractTask> {
    let opts_in = options.unwrap_or_default();
    let want_text = opts_in.output_text.unwrap_or(false);
    let preflight_failure = opts_in.max_input_size.and_then(|limit| {
        let limit = limit as usize;
        (html.len() > limit).then(|| too_large_result(html.len(), limit))
    });
    let opts = map_options(&opts_in);
    AsyncTask::new(ExtractTask {
        html,
        opts,
        want_text,
        preflight_failure,
    })
}

/// Library version, exposed to JS callers for debugging / telemetry.
#[napi]
pub fn version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}
