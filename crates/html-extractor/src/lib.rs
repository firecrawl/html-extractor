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

pub use types::{
    Decision, ExtractError, ExtractOptions, ExtractResult, ExtractStats, Metadata, PageType,
};

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

    // Stage 4: fallback if selected subtree is too small or absent. Three
    // triggers:
    //   (a) Stage 3 didn't pick anything (selected_root is None)
    //   (b) The picked subtree's text is below the user-configured minimum
    //   (c) The picked subtree is *suspiciously* small relative to the body
    //       text — typically means the scored walk locked onto an intro
    //       paragraph or product-nav and missed the real (link-dense, table-
    //       layout, or component-grid) main content. We re-run with the
    //       fallback chain and keep whichever found more text-excluding-links.
    let kept_text_len = selected_root
        .map(|idx| tree.text_len_excluding_links(idx))
        .unwrap_or(0);
    let min_len = options.min_extraction_length;
    let body_text_len = if tree.body != usize::MAX {
        tree.text_len_excluding_links(tree.body)
    } else {
        0
    };
    // Suspicious-pick threshold: chosen subtree has < 15% of body text-
    // excluding-links AND body text is large enough (≥200 chars) that the
    // disparity is meaningful. Catches a class of failure where the scored
    // walk locks onto an intro paragraph or a small component-grid and misses
    // the substantive main content elsewhere on the page (typically when the
    // real content has high link density and the wider penalty regime drives
    // its aggregate negative). 15% is empirical, tuned against a small real-
    // world corpus.
    let suspiciously_small_excl_links =
        body_text_len >= 200 && kept_text_len * 100 < body_text_len * 15;
    // Link-heavy variant: when nearly all body text IS link text (table-
    // layout listings of all-anchor rows), text_len_excluding_links is near-
    // zero for both the body and the chosen subtree, so the excl-links ratio
    // above can't detect the disparity. Use full-text on both sides with a
    // tighter 5% threshold and a minimum 1000-char body to avoid false-
    // positive triggers on small marketing pages.
    let kept_full_text = selected_root
        .map(|idx| tree.full_text(idx).chars().count())
        .unwrap_or(0);
    let body_full_text = if tree.body != usize::MAX {
        tree.full_text(tree.body).chars().count()
    } else {
        0
    };
    let suspiciously_small_full =
        body_full_text >= 1000 && kept_full_text * 100 < body_full_text * 5;
    let suspiciously_small = suspiciously_small_excl_links || suspiciously_small_full;
    let (final_root, quality, used_fallback) = if let Some(idx) = selected_root {
        if kept_text_len < min_len {
            // (b): too short to be useful — fall through.
            let (fb_root, q) = fallback::fallback(&tree, options);
            (fb_root.or(Some(idx)), q.max(0.15), true)
        } else if suspiciously_small {
            // (c): try the fallback chain and pick whichever produced more
            // text-excluding-links content. We accept the fallback if it
            // found 1.5x or more — tighter ratios were too conservative on
            // pages where Stage 3's pick was small but fallback's was only
            // moderately bigger.
            let (fb_root, fb_q) = fallback::fallback(&tree, options);
            let fb_text = fb_root
                .map(|i| tree.text_len_excluding_links(i))
                .unwrap_or(0);
            if fb_text * 2 > kept_text_len * 3 {
                (fb_root, fb_q.max(0.2), true)
            } else {
                (
                    Some(idx),
                    confidence_from_score(score, kept_text_len),
                    false,
                )
            }
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
    let (markdown, text_chars, decisions) = if let Some(root) = final_root {
        let cleaned = if matches!(page_type, PageType::Listing | PageType::Collection) {
            clean::post_clean_lenient_links(&tree, root, options)
        } else {
            clean::post_clean(&tree, root, options)
        };
        let (markdown, text_chars) = render::render(&tree, &cleaned, options);
        // Ledger: the kept main container followed by each dropped block.
        let decisions = if options.output_decisions {
            let mut v = Vec::with_capacity(cleaned.decisions.len() + 1);
            v.push(Decision {
                selector: tree.get(root).selector(),
                score: 1.0,
                kept: true,
                confidence: quality,
            });
            v.extend(cleaned.decisions);
            Some(v)
        } else {
            None
        };
        (markdown, text_chars, decisions)
    } else {
        (String::new(), 0, None)
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
        decisions,
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
