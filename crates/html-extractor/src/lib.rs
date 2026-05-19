//! `html-extractor` — fast HTML main-content extractor.
//!
//! See `README.md` and `SPEC.md` for the full overview. Public entry point:
//! [`extract`].

#![warn(missing_docs)]
#![forbid(unsafe_code)]

mod classifier;
mod clean;
mod fallback;
mod metadata;
mod parser;
mod render;
mod scoring;
mod tree;
mod types;

pub use types::{ExtractError, ExtractOptions, ExtractResult, ExtractStats, Metadata, PageType};

/// Extract the main content of an HTML document, returning a structured
/// [`ExtractResult`] containing markdown, page-type, metadata, and a confidence
/// score.
///
/// See `SPEC.md` for the full contract. Common usage:
///
/// ```rust
/// use html_extractor::{extract, ExtractOptions};
///
/// let html = r#"<html><body><article><h1>Hi</h1><p>Hello world. This is some article body text long enough to pass the minimum extraction threshold.</p></article></body></html>"#;
/// let out = extract(html, &ExtractOptions::default()).unwrap();
/// assert!(out.markdown.contains("Hello world"));
/// ```
pub fn extract(html: &str, options: &ExtractOptions) -> Result<ExtractResult, ExtractError> {
    if options.favor_precision && options.favor_recall {
        return Err(ExtractError::ConflictingOptions(
            "favor_precision and favor_recall are mutually exclusive",
        ));
    }
    if html.trim().is_empty() {
        return Ok(ExtractResult::empty(ExtractError::EmptyInput));
    }

    // Stage 0: parse
    let raw_tree = match parser::parse(html) {
        Ok(t) => t,
        Err(e) => return Ok(ExtractResult::empty(e)),
    };

    // Metadata is harvested from the raw tree before any pruning (Stage 5
    // logically, but earlier in code because `<head>` is dropped in Stage 1).
    let metadata = if options.include_metadata {
        metadata::extract(&raw_tree)
    } else {
        Metadata::default()
    };

    // Stage 2: classify
    let (page_type, _classify_conf) =
        classifier::classify(&raw_tree, options.url.as_deref(), &metadata);
    let page_type = options.page_type_override.unwrap_or(page_type);

    // Stage 1: pre-clean
    let mut tree = raw_tree;
    clean::pre_clean(&mut tree, options);

    // Stage 3: score & select main subtree, with fast-paths.
    let profile = scoring::profile_for(page_type);
    let (selected_root, score) = scoring::select_main(&tree, &profile, options);

    // Stage 4: fallback if selected subtree is too small or absent.
    let kept_text_len = selected_root
        .map(|idx| tree.text_len_excluding_links(idx))
        .unwrap_or(0);
    let min_len = options.min_extraction_length;
    let (final_root, quality, used_fallback) = if let Some(idx) = selected_root {
        if kept_text_len < min_len {
            let (fb_root, q) = fallback::fallback(&tree, options);
            (fb_root.or(Some(idx)), q.max(0.15), true)
        } else {
            (
                Some(idx),
                confidence_from_score(score, kept_text_len),
                false,
            )
        }
    } else {
        let (fb_root, q) = fallback::fallback(&tree, options);
        (fb_root, q.max(0.1), true)
    };

    // Stage 5: post-clean within the kept subtree, then render.
    let (markdown, text_chars) = if let Some(root) = final_root {
        let cleaned = if matches!(page_type, PageType::Listing | PageType::Collection) {
            clean::post_clean_lenient_links(&tree, root, options)
        } else {
            clean::post_clean(&tree, root, options)
        };
        render::render(&tree, &cleaned, options)
    } else {
        (String::new(), 0)
    };

    let stats = ExtractStats {
        text_chars,
        element_count: tree.len(),
        used_fallback,
        page_type,
    };

    let language = metadata.language.clone();
    Ok(ExtractResult {
        markdown,
        text: None,
        page_type,
        extraction_quality: if text_chars == 0 { 0.0 } else { quality },
        language,
        metadata: if options.include_metadata {
            Some(metadata)
        } else {
            None
        },
        decisions: None,
        stats: Some(stats),
        error_reason: None,
    })
}

fn confidence_from_score(score: f32, text_len: usize) -> f32 {
    // Hand-tuned: a normal article scores ~50–400 in our scoring units. Map
    // log-ish to [0, 1].
    let s = (score.max(0.0) / 200.0).min(1.0);
    let l = (text_len as f32 / 800.0).min(1.0);
    (0.55 * s + 0.45 * l).clamp(0.0, 1.0)
}
