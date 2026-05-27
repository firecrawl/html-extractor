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
/// value (mirrors trafilatura's `MANUALLY_STRIPPED`). We keep tbody/thead/tfoot
/// in the tree because the table renderer descends through them; folding them
/// to spans here would break table layout.
const STRIP_TAGS: &[&str] = &[
    "abbr", "acronym", "address", "bdi", "bdo", "big", "cite", "data", "dfn", "font", "hgroup",
    "ins", "mark", "ruby", "small", "time", "noindex",
];

static HIDDEN_STYLE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)display\s*:\s*none|visibility\s*:\s*hidden").unwrap());

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
        if elem.has_attr("hidden")
            || elem
                .attr("aria-hidden")
                .map(|v| v == "true")
                .unwrap_or(false)
        {
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
            tree.get_mut(idx).tag = "span".into();
        }
    }
}

/// Stage 5 post-clean: applied to the subtree chosen as main content.
///
/// Returns the index of the cleaned subtree (same as input — we mutate the
/// shared tree). Drops well-known chrome blocks inside the selection by class
/// hints + link-density heuristics.
pub(crate) fn post_clean(tree: &Tree, root: usize, options: &ExtractOptions) -> CleanedRoot {
    post_clean_inner(tree, root, options, true)
}

/// Like [`post_clean`] but skips the link-density filter — used when the page
/// type is `Listing` / `Collection`, where the list IS the content.
pub(crate) fn post_clean_lenient_links(
    tree: &Tree,
    root: usize,
    options: &ExtractOptions,
) -> CleanedRoot {
    post_clean_inner(tree, root, options, false)
}

fn post_clean_inner(
    tree: &Tree,
    root: usize,
    options: &ExtractOptions,
    apply_link_density: bool,
) -> CleanedRoot {
    // We don't mutate `tree` here; instead we collect a set of "skipped"
    // descendant indices that the renderer will respect. This keeps `tree`
    // shareable with other passes (e.g. for fallback retries).
    let mut skip: std::collections::HashSet<usize> = std::collections::HashSet::new();
    // Subtree text/link-char metrics for the whole selection, computed in one
    // post-order pass. Looking these up is O(1); the previous code called
    // `full_text` per node, which re-walked each subtree (O(N²)).
    let metrics = tree.subtree_text_metrics(root);
    // Two guards against over-stripping the kept subtree:
    //
    //   * Per-element dominant guard: never strip a single descendant that
    //     holds ≥50% of the kept subtree's text. Class names like
    //     `single-post-content` or `entry-meta` can match chrome even though
    //     they wrap the actual body.
    //
    //   * Cumulative budget: stop stripping after the running total of
    //     stripped text would exceed 60% of the kept subtree. Some site
    //     templates (e.g. Elementor's `class="elementor-widget"` on every
    //     content block) have many small chrome-class siblings that each pass
    //     the per-element guard, but together would erase the whole body.
    //     This budget catches that case.
    let root_text_len = metrics.chars[root];
    // Per-element dominant threshold lowered from 50% → 30% so subtrees that
    // hold a substantial-but-not-majority share of the body (e.g. the article
    // body wrapped in a hashed-class container, ~30-45% of root_text on pages
    // with sidebars and footers) are also protected from chrome-class strips.
    let dominant_threshold = ((root_text_len * 3) / 10).max(1);
    let strip_budget = (root_text_len * 6) / 10; // 60%
    let mut stripped_total: usize = 0;
    let mut stack = vec![root];
    while let Some(idx) = stack.pop() {
        let elem = tree.get(idx);
        if elem.tag == "_dropped_" {
            continue;
        }
        let idx_text = metrics.chars[idx];
        let is_dominant = idx != root && idx_text >= dominant_threshold;
        let would_exceed_budget =
            idx != root && idx_text > 0 && stripped_total + idx_text > strip_budget;

        let needle = elem.class_id_lower();
        let chrome_hit = !needle.is_empty() && is_chrome(needle);
        let share_hit = !needle.is_empty() && is_share_or_ad(needle);
        let link_hit = apply_link_density
            && !options.favor_recall
            && matches!(elem.tag.as_str(), "div" | "ul" | "ol" | "p" | "section")
            && is_link_dense(idx_text, metrics.link_chars[idx], options.favor_precision);

        if (chrome_hit || share_hit || link_hit)
            && idx != root
            && !is_dominant
            && !would_exceed_budget
        {
            skip.insert(idx);
            stripped_total += idx_text;
            // don't descend into a dropped subtree
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
        "(?i)(^|[ _-])(nav(bar)?|footer|sidebar|breadcrumb|cookie|consent|newsletter|\
         subnav|banner|tag-list|related|elated|widget|attachment|user-(info|profile)|\
         byline(-?author)?|rating|teaser|paywall|paid-?content|outbrain|taboola|criteo|\
         most-?popular|popular-?(posts|articles|stories)?|trending(-?(now|posts))?|\
         recommend(ed|ation)s?|mol-factbox|comments?-?(title|list|form|respond|cta)?|\
         kommentare?|signin|nocomments|reply-?|hide-print|noprint|skip-?link|\
         site-(header|footer)|page-(header|footer)|menu|navigation|toolbar|\
         author-?(box|info|bio|card|meta|profile|details)|about-?(the-)?author|\
         more-?(articles|stories|posts|reads|like-this|from|topic|on-this)|\
         subscribe(-?(form|box|cta))?|signup(-?form)?|abonniere?n?|\
         post-?meta|entry-?meta|article-?meta|\
         categor(y|ies|ien)|kategorien?|cats|\
         pagination|page-?nav(igation)?|post-?nav(igation)?|\
         prev(ious)?-?(post|page|article)?|next-?(post|page|article)|\
         vorherige?s|nachste?r?|naechste?r?|\
         see-?also|read-?(also|more|further|next)|further-?(reading|articles)|\
         zobacz-?(takze|wi[ęe]cej)|czytaj-?(te[żz]|wi[ęe]cej)|\
         stay-?(up-?to-?date|informed|connected)|follow-?(us|on)|\
         post-?(footer|share|info)|article-?(footer|share|info|copyright)|\
         entry-?(footer|share|utility|tools)|\
         textwidget|widgettitle|widgetcontainer|widget-?content|widget-?header|\
         morelinks|more-?links?|\
         published|dt-published|posted-on|post-?date|article-?date|publish-?date|\
         text-?(dimmed|muted)|\
         box-?description|module-?box|asset-?box|info-?box|\
         e-content|p-summary)($|[ _-])",
    )
    .unwrap()
});

static SHARE_AD_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        "(?i)(^|[ _-])(share-?(this|to|on|button|bar|box)?|social(-?(share|links|icons|bar))?|\
         ad(s|vert(isement)?)?|sponsor(ed)?|promo|jp-(post-flair|relatedposts)|\
         dpsp-content|popover|paywall|gdpr|cookie-(bar|notice|banner)|\
         email-?(this|article|post|to-?a-?friend)|forward-?this|tell-?a-?friend|\
         print-?(this|article|page)|bookmark|save-?(for-?later|article|post))($|[ _-])",
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
/// `link_density_test` semantics, simplified for our scoring units. Takes
/// precomputed subtree totals (see [`Tree::subtree_text_metrics`]) so callers
/// don't re-walk the subtree per node.
fn is_link_dense(total_chars: usize, link_chars: usize, favor_precision: bool) -> bool {
    if total_chars < 30 {
        return false;
    }
    // Default threshold lowered from 0.7 to 0.6 to be more aggressive on
    // related-stories rails and link-heavy navigation widgets that pass the
    // chrome-class filter but still hold mostly links rather than prose.
    let threshold = if favor_precision { 0.5 } else { 0.6 };
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
