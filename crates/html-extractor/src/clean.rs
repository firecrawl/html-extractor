//! Stage 1 (pre-clean) and Stage 5 (post-clean) tree pruning.
//!
//! Tag lists mirror trafilatura's `MANUALLY_CLEANED` and `MANUALLY_STRIPPED`
//! in spirit, not character-for-character. Adjustments:
//! - We don't fold image-related cleanup into pre-clean; that's an option in
//!   the public API instead.
//! - Hidden-element detection runs at clean time rather than scoring time so
//!   downstream walks see a smaller tree.

use once_cell::sync::Lazy;
use regex::Regex;

use crate::tree::Tree;
use crate::types::ExtractOptions;

/// Tags whose entire subtree is never content.
const KILL_TAGS: &[&str] = &[
    "script", "style", "noscript", "head", "template", "iframe", "form", "object", "embed",
    "applet", "audio", "video", "canvas", "svg", "math", "menu", "menuitem", "dialog", "fieldset",
    "frame", "frameset", "input", "select", "textarea", "button", "label", "legend", "marquee",
    "blink", "datalist", "optgroup", "option", "progress", "param", "source", "track", "use",
    "area", "link", "map",
];

/// Tags considered chrome in most page types. We don't always drop them in
/// pre-clean (some pages put real content in `<aside>`), so this list is only
/// used when `favor_precision` is set or as part of post-clean.
const PRECISION_KILL_TAGS: &[&str] = &["aside", "nav", "footer", "header"];

/// Tags whose own text is preserved but the wrapper itself adds no semantic
/// value (mirrors trafilatura's `MANUALLY_STRIPPED`).
const STRIP_TAGS: &[&str] = &[
    "abbr", "acronym", "address", "bdi", "bdo", "big", "cite", "data", "dfn", "font", "hgroup",
    "ins", "mark", "ruby", "small", "tbody", "thead", "tfoot", "time", "noindex",
];

static HIDDEN_STYLE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)display\s*:\s*none|visibility\s*:\s*hidden").unwrap()
});

/// Stage 1: pre-clean. Drops `<script>`, `<style>`, hidden elements, etc.
pub(crate) fn pre_clean(tree: &mut Tree, options: &ExtractOptions) {
    let root = tree.root;
    let mut to_drop: Vec<usize> = Vec::new();
    let mut to_strip: Vec<usize> = Vec::new();
    tree.walk_pre(root, |idx| {
        let elem = tree.get(idx);
        if elem.tag == "_dropped_" {
            return false;
        }
        // Hidden via inline style or aria-hidden.
        if let Some(style) = elem.attr("style") {
            if HIDDEN_STYLE.is_match(style) {
                to_drop.push(idx);
                return false;
            }
        }
        if elem.has_attr("hidden") || elem.attr("aria-hidden").map(|v| v == "true").unwrap_or(false) {
            // Edge case: aria-hidden on a link inside a card is content
            // duplication, but per ALGORITHM.md we accept the trafilatura
            // simplification — drop.
            to_drop.push(idx);
            return false;
        }
        if KILL_TAGS.contains(&elem.tag.as_str()) {
            to_drop.push(idx);
            return false;
        }
        if options.favor_precision && PRECISION_KILL_TAGS.contains(&elem.tag.as_str()) {
            to_drop.push(idx);
            return false;
        }
        if !options.include_images && elem.tag == "img" {
            to_drop.push(idx);
            return false;
        }
        if !options.include_tables && (elem.tag == "table") {
            to_drop.push(idx);
            return false;
        }
        if STRIP_TAGS.contains(&elem.tag.as_str()) {
            to_strip.push(idx);
        }
        true
    });
    for idx in to_drop {
        tree.drop_subtree(idx);
    }
    // STRIP_TAGS: keep their children but lower their own visibility — easiest
    // is to rename them to `span` so the renderer treats them as inline
    // formatting wrappers.
    for idx in to_strip {
        if tree.get(idx).tag != "_dropped_" {
            tree.get_mut(idx).tag = "span".to_string();
        }
    }
}

/// Stage 5 post-clean: applied to the subtree chosen as main content.
///
/// Returns the index of the cleaned subtree (same as input — we mutate the
/// shared tree). Drops well-known chrome blocks inside the selection by class
/// hints + link-density heuristics.
pub(crate) fn post_clean(
    tree: &Tree,
    root: usize,
    options: &ExtractOptions,
) -> CleanedRoot {
    // We don't mutate `tree` here; instead we collect a set of "skipped"
    // descendant indices that the renderer will respect. This keeps `tree`
    // shareable with other passes (e.g. for fallback retries).
    let mut skip: std::collections::HashSet<usize> = std::collections::HashSet::new();
    tree.walk_subtree_text(root, &mut |elem| {
        // Cheap chrome match by class/id.
        let needle = elem.class_id_lower();
        if !needle.is_empty() && is_chrome(&needle) {
            // Defer the actual drop decision until we look at the descendant
            // count, since some "header" classes wrap whole sections that
            // include real content.
            return true;
        }
        true
    });
    // Iterate again with index access this time (so we can build `skip`).
    let mut stack = vec![root];
    while let Some(idx) = stack.pop() {
        let elem = tree.get(idx);
        if elem.tag == "_dropped_" {
            continue;
        }
        let needle = elem.class_id_lower();
        if !needle.is_empty() && is_chrome(&needle) && idx != root {
            skip.insert(idx);
            // don't descend into a dropped subtree
            continue;
        }
        // Drop inline ads / share buttons by class even when chrome regex
        // misses them.
        if !needle.is_empty() && is_share_or_ad(&needle) && idx != root {
            skip.insert(idx);
            continue;
        }
        // Link-density filter for div/list/p — see trafilatura
        // `delete_by_link_density`.
        if !options.favor_recall
            && matches!(elem.tag.as_str(), "div" | "ul" | "ol" | "p" | "section")
            && is_link_dense(tree, idx, options.favor_precision)
            && idx != root
        {
            skip.insert(idx);
            continue;
        }
        for &c in &elem.children {
            stack.push(c);
        }
    }
    CleanedRoot { root, skip }
}

/// Result of post-clean: a root index plus a set of subtrees to skip.
pub(crate) struct CleanedRoot {
    pub root: usize,
    pub skip: std::collections::HashSet<usize>,
}

static CHROME_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        "(?i)(^|[ _-])(nav(bar)?|footer|sidebar|breadcrumb|cookie|consent|newsletter|subnav|banner|tag-list|related|elated|widget|attachment|user-(info|profile)|byline|rating|teaser|paywall|paid-?content|outbrain|taboola|criteo|most-?popular|mol-factbox|comments?-?(title|list)|signin|nocomments|reply-?|hide-print|noprint|skip-?link|site-(header|footer)|page-(header|footer)|menu|navigation|toolbar)($|[ _-])",
    )
    .unwrap()
});

static SHARE_AD_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        "(?i)(^|[ _-])(share-?(this|to|on)?|social(-?(share|links))?|ad(s|vert(isement)?)?|sponsor(ed)?|promo|jp-(post-flair|relatedposts)|dpsp-content|popover|paywall|gdpr|cookie-(bar|notice|banner))($|[ _-])",
    )
    .unwrap()
});

pub(crate) fn is_chrome(class_id: &str) -> bool {
    CHROME_RE.is_match(class_id)
}

pub(crate) fn is_share_or_ad(class_id: &str) -> bool {
    SHARE_AD_RE.is_match(class_id)
}

/// Per-element link-density check. Mirrors trafilatura's
/// `link_density_test` semantics, simplified for our scoring units.
fn is_link_dense(tree: &Tree, idx: usize, favor_precision: bool) -> bool {
    let total = tree.full_text(idx);
    let total_chars = total.chars().count();
    if total_chars < 30 {
        return false;
    }
    let mut link_chars = 0usize;
    tree.walk_subtree_text(idx, &mut |elem| {
        if elem.tag == "a" {
            link_chars += elem.own_text.chars().count();
        }
        elem.tag != "_dropped_"
    });
    let threshold = if favor_precision { 0.5 } else { 0.7 };
    let ratio = link_chars as f32 / total_chars as f32;
    if total_chars < 1000 {
        ratio > threshold
    } else {
        ratio > threshold - 0.15
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chrome_regex_matches_known_chrome_classes() {
        assert!(is_chrome("site-footer"));
        assert!(is_chrome("primary-nav"));
        assert!(is_chrome("sidebar-widget"));
        assert!(is_chrome("breadcrumb-list"));
        assert!(is_chrome("cookie-banner"));
        assert!(is_chrome("related elated"));
    }

    #[test]
    fn chrome_regex_ignores_content_classes() {
        assert!(!is_chrome("article-body"));
        assert!(!is_chrome("entry-content"));
        assert!(!is_chrome("post-text"));
    }

    #[test]
    fn share_ad_regex_matches() {
        assert!(is_share_or_ad("share-buttons"));
        assert!(is_share_or_ad("social-links"));
        assert!(is_share_or_ad("sponsor"));
        assert!(is_share_or_ad("ad"));
        assert!(is_share_or_ad("advertisement"));
    }
}
