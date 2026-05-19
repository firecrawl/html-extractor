//! Stage 3 — per-element scoring and tree-walk subtree selection.
//!
//! Weights are hand-tuned with intent — see `DECISIONS.md` D5. The features
//! correspond 1:1 to the table in `ALGORITHM.md` Stage 3.

use once_cell::sync::Lazy;
use regex::Regex;

use crate::tree::Tree;
use crate::types::{ExtractOptions, PageType};

/// Scoring profile. Each page type can override the weights.
#[derive(Debug, Clone)]
pub(crate) struct ScoringProfile {
    pub w_text_length: f32,
    pub w_text_density: f32,
    pub w_link_density: f32,
    pub w_tag_weight: f32,
    pub w_class_hint: f32,
    pub w_position: f32,
    pub w_parent_chain: f32,
    /// Per-tag bonus/penalty.
    pub tag_weight: fn(&str) -> f32,
}

fn default_tag_weight(tag: &str) -> f32 {
    match tag {
        "article" | "main" => 8.0,
        "section" => 2.0,
        "p" => 3.0,
        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => 2.0,
        "blockquote" => 2.0,
        "pre" | "code" => 1.5,
        "ul" | "ol" | "li" | "dl" => 0.5,
        "table" => 0.0,
        "div" => 0.0,
        "nav" | "aside" | "footer" | "header" => -6.0,
        "form" | "noscript" | "iframe" => -10.0,
        _ => 0.0,
    }
}

fn forum_tag_weight(tag: &str) -> f32 {
    match tag {
        "article" => 6.0,
        "section" => 3.0,
        // Forums often wrap each post in <li> / <div role="post">; reward
        // siblings rather than penalising them.
        "li" => 2.0,
        _ => default_tag_weight(tag),
    }
}

fn listing_tag_weight(tag: &str) -> f32 {
    match tag {
        // The list itself is the content — reward it.
        "ul" | "ol" => 6.0,
        "li" => 2.0,
        "article" | "main" => 4.0,
        _ => default_tag_weight(tag),
    }
}

fn product_tag_weight(tag: &str) -> f32 {
    match tag {
        "article" => 4.0,
        "main" => 6.0,
        "table" => 1.0, // attribute tables are often real content here
        _ => default_tag_weight(tag),
    }
}

fn docs_tag_weight(tag: &str) -> f32 {
    match tag {
        "pre" | "code" => 4.0,
        "h2" | "h3" => 3.0,
        _ => default_tag_weight(tag),
    }
}

/// Pick the scoring profile for a page type.
pub(crate) fn profile_for(pt: PageType) -> ScoringProfile {
    let base = ScoringProfile {
        w_text_length: 1.0,
        w_text_density: 0.35,
        w_link_density: 1.8,
        w_tag_weight: 1.0,
        w_class_hint: 1.0,
        w_position: 0.4,
        w_parent_chain: 0.6,
        tag_weight: default_tag_weight,
    };
    match pt {
        PageType::Article | PageType::Other | PageType::Service | PageType::Collection => base,
        PageType::Forum => ScoringProfile {
            w_link_density: 1.0,
            tag_weight: forum_tag_weight,
            ..base
        },
        PageType::Listing => ScoringProfile {
            w_link_density: 0.6,
            w_text_density: 0.2,
            tag_weight: listing_tag_weight,
            ..base
        },
        PageType::Product => ScoringProfile {
            w_link_density: 1.0,
            tag_weight: product_tag_weight,
            ..base
        },
        PageType::Documentation => ScoringProfile {
            w_text_density: 0.3,
            tag_weight: docs_tag_weight,
            ..base
        },
    }
}

static CONTENT_HINT_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        "(?i)(\\b|_|-)(article(-?(body|content|inner|main|text|wrap))?|post(-?(body|content|entry|text))?|entry(-?content)?|main(-?content)?|content(-?(body|main))?|story(-?body)?|page-?content|text-?content|body-?text|prose|markdown-?body)\\b",
    )
    .unwrap()
});

static CHROME_HINT_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        "(?i)(\\b|_|-)(nav(igation|bar)?|menu|footer|sidebar|aside|widget|promo|ad(s|vert)?|sponsor|byline|share|social|related|comments?|cookie|consent|popup|modal|banner|tag-list|breadcrumbs?|tools?|toolbar)\\b",
    )
    .unwrap()
});

/// Per-element score components (own).
#[derive(Debug, Clone, Default)]
struct OwnScore {
    /// log(text_chars + 1)
    text_length_term: f32,
    /// text_chars / (descendant_tag_count + 1)
    text_density_term: f32,
    /// link_chars / text_chars (clamped 0..1)
    link_density: f32,
    /// (profile.tag_weight)(tag)
    tag_weight_term: f32,
    /// class_hint signal (+ for content, − for chrome)
    class_hint_term: f32,
    /// position-based gentle nudge in [-1, 1]
    position_term: f32,
    /// ancestor bonus accumulated by walking up to root
    parent_chain_term: f32,
}

/// Run Stage 3, returning the index of the chosen subtree (if any) plus its
/// aggregate score.
pub(crate) fn select_main(
    tree: &Tree,
    profile: &ScoringProfile,
    options: &ExtractOptions,
) -> (Option<usize>, f32) {
    let root = tree.body;
    if tree.nodes.is_empty() || root == usize::MAX {
        return (None, 0.0);
    }

    // 1. Fast-path: a single `<main>` or `<article>` with a sensible amount
    //    of text passes through directly.
    if let Some(fast) = fast_path(tree, root, options) {
        let aggr = score_subtree_quick(tree, fast, profile);
        return (Some(fast), aggr);
    }

    // 2. Score all elements bottom-up.
    let n = tree.nodes.len();
    let mut own_scores: Vec<OwnScore> = vec![OwnScore::default(); n];
    let mut aggregate: Vec<f32> = vec![0.0; n];

    // Pre-compute counters per element using a single post-order pass.
    let body_depth = depth_of(tree, root);

    tree.walk_post(root, |idx| {
        let elem = tree.get(idx);
        if elem.tag == "_dropped_" {
            return;
        }

        // Text counters via cheap walk over our own subtree text.
        let mut text_chars = 0usize;
        let mut link_chars = 0usize;
        let mut tag_count = 0usize;
        tree.walk_subtree_text(idx, &mut |e| {
            if e.tag == "_dropped_" {
                return false;
            }
            tag_count += 1;
            let chars = e.own_text.chars().count();
            if e.tag == "a" {
                link_chars += chars;
            }
            text_chars += chars;
            true
        });
        let descendant_tag_count = tag_count.saturating_sub(1);

        let text_length_term = ((text_chars as f32) + 1.0).ln();
        let text_density_term = if descendant_tag_count == 0 {
            text_chars as f32
        } else {
            text_chars as f32 / (descendant_tag_count as f32 + 1.0)
        };
        let link_density = if text_chars == 0 {
            0.0
        } else {
            (link_chars as f32 / text_chars as f32).clamp(0.0, 1.0)
        };
        let tag_weight_term = (profile.tag_weight)(elem.tag.as_str());
        let class_id_lc = elem.class_id_lower();
        let class_hint_term = class_hint_score(&class_id_lc);

        // Position term: 0 at the edges, ~1 in the middle 60% of the document.
        let position_term = position_score(tree, root, idx, body_depth);

        own_scores[idx] = OwnScore {
            text_length_term,
            text_density_term,
            link_density,
            tag_weight_term,
            class_hint_term,
            position_term,
            parent_chain_term: 0.0, // filled in below
        };
    });

    // Parent-chain bonus pass: a child inherits some of its ancestors'
    // tag/class hints.
    {
        let mut stack: Vec<(usize, f32)> = vec![(root, 0.0)];
        while let Some((idx, inherited)) = stack.pop() {
            own_scores[idx].parent_chain_term = inherited;
            let own = (profile.tag_weight)(tree.get(idx).tag.as_str())
                + class_hint_score(&tree.get(idx).class_id_lower());
            // Inherited bonus decays as we descend.
            let next = (inherited * 0.7) + (own * 0.3);
            for &c in &tree.get(idx).children {
                if tree.get(c).tag == "_dropped_" {
                    continue;
                }
                stack.push((c, next));
            }
        }
    }

    // Compute own raw score per element.
    let own_raw: Vec<f32> = (0..n)
        .map(|i| {
            let s = &own_scores[i];
            let mut r = profile.w_text_length * s.text_length_term
                + profile.w_text_density * s.text_density_term
                + profile.w_tag_weight * s.tag_weight_term
                + profile.w_class_hint * s.class_hint_term
                + profile.w_position * s.position_term
                + profile.w_parent_chain * s.parent_chain_term;
            // Link-density penalty
            r -= profile.w_link_density * s.link_density * 20.0;
            r
        })
        .collect();

    // Aggregate post-order: aggregate[parent] = own_raw[parent] + sum(aggregate[child])
    tree.walk_post(root, |idx| {
        let elem = tree.get(idx);
        if elem.tag == "_dropped_" {
            return;
        }
        let mut sum = own_raw[idx];
        for &c in &elem.children {
            sum += aggregate[c];
        }
        aggregate[idx] = sum;
    });

    // Pick the best subtree subject to constraints:
    //  - text length above `min_extraction_length`
    //  - not the body element unless everything else is worse
    //  - prefer deeper / more specific subtree when scores are tied
    let mut best: Option<(usize, f32, usize)> = None; // (idx, score, text_len)
    for idx in 0..n {
        if tree.get(idx).tag == "_dropped_" {
            continue;
        }
        if idx == tree.root {
            continue;
        }
        let text_len = own_scores[idx].text_length_term; // proxy
        if text_len < ((options.min_extraction_length as f32 + 1.0).ln()) {
            continue;
        }
        let mut score = aggregate[idx];
        // Prefer non-<body>: only accept <body> if nothing else clears the bar.
        let is_body = idx == tree.body;
        if is_body {
            score -= 20.0;
        }
        match best {
            None => best = Some((idx, score, text_len.round() as usize)),
            Some((_, bs, _)) if score > bs + 0.5 => {
                best = Some((idx, score, text_len.round() as usize))
            }
            _ => {}
        }
    }
    let chosen = best.map(|t| t.0);
    let final_score = best.map(|t| t.1).unwrap_or(0.0);
    (chosen, final_score)
}

fn fast_path(tree: &Tree, body: usize, options: &ExtractOptions) -> Option<usize> {
    // Try (in order): a <main>, an <article>, then class/id heuristics.
    let mut candidate = None;
    tree.walk_pre(body, |idx| {
        if candidate.is_some() {
            return false;
        }
        let elem = tree.get(idx);
        if elem.tag == "_dropped_" {
            return false;
        }
        if elem.tag == "main" || elem.tag == "article" {
            // For pages that are mostly link-text (listings), the
            // text-excluding-links check is too strict; fall back to total
            // text length.
            let text = tree
                .text_len_excluding_links(idx)
                .max(tree.full_text(idx).chars().count() / 4);
            if text >= options.min_extraction_length.max(120) {
                candidate = Some(idx);
                return false;
            }
        }
        true
    });
    if candidate.is_some() {
        return candidate;
    }
    // Class/id fast-path
    tree.walk_pre(body, |idx| {
        if candidate.is_some() {
            return false;
        }
        let elem = tree.get(idx);
        if elem.tag == "_dropped_" {
            return false;
        }
        let needle = elem.class_id_lower();
        if !needle.is_empty() && CONTENT_HINT_RE.is_match(&needle) {
            let text = tree.text_len_excluding_links(idx);
            if text >= options.min_extraction_length.max(120) {
                candidate = Some(idx);
                return false;
            }
        }
        true
    });
    candidate
}

fn class_hint_score(class_id_lc: &str) -> f32 {
    if class_id_lc.is_empty() {
        return 0.0;
    }
    let mut s = 0.0;
    if CONTENT_HINT_RE.is_match(class_id_lc) {
        s += 5.0;
    }
    if CHROME_HINT_RE.is_match(class_id_lc) {
        s -= 5.0;
    }
    s
}

fn depth_of(tree: &Tree, root: usize) -> usize {
    let mut max_d = 0;
    let mut stack: Vec<(usize, usize)> = vec![(root, 0)];
    while let Some((idx, d)) = stack.pop() {
        if d > max_d {
            max_d = d;
        }
        for &c in &tree.get(idx).children {
            stack.push((c, d + 1));
        }
    }
    max_d
}

fn position_score(tree: &Tree, body: usize, idx: usize, _max_depth: usize) -> f32 {
    // Walk up to body counting ancestors and siblings encountered. Cheap
    // proxy: index in the parent's child list combined with depth.
    let mut current = idx;
    let mut sibling_pos = 0.5_f32;
    let mut found = false;
    while current != body && current != usize::MAX {
        let parent = tree.get(current).parent;
        if parent == usize::MAX {
            break;
        }
        let siblings = &tree.get(parent).children;
        if let Some(pos) = siblings.iter().position(|&c| c == current) {
            let total = siblings.len().max(1) as f32;
            let frac = (pos as f32 + 0.5) / total;
            // Middle siblings score higher than edge siblings.
            sibling_pos = 1.0 - (frac - 0.5).abs() * 2.0;
            found = true;
            break;
        }
        current = parent;
    }
    if found {
        sibling_pos
    } else {
        0.0
    }
}

fn score_subtree_quick(tree: &Tree, idx: usize, profile: &ScoringProfile) -> f32 {
    // Used only as the return-value score for the fast-path. Approximate the
    // subtree score via text size + tag weight.
    let text = tree.text_len_excluding_links(idx);
    let tag = (profile.tag_weight)(tree.get(idx).tag.as_str());
    profile.w_text_length * ((text as f32) + 1.0).ln() + profile.w_tag_weight * tag
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tag_weight_table_matches_design() {
        assert!(default_tag_weight("article") > 0.0);
        assert!(default_tag_weight("main") > 0.0);
        assert!(default_tag_weight("nav") < 0.0);
        assert!(default_tag_weight("aside") < 0.0);
        assert!(default_tag_weight("footer") < 0.0);
        assert_eq!(default_tag_weight("div"), 0.0);
        assert!(default_tag_weight("p") > 0.0);
    }

    #[test]
    fn class_hint_recognizes_content_class() {
        assert!(class_hint_score("article-body") > 0.0);
        assert!(class_hint_score("entry-content") > 0.0);
        assert!(class_hint_score("post-text") > 0.0);
    }

    #[test]
    fn class_hint_penalizes_chrome_class() {
        assert!(class_hint_score("site-footer") < 0.0);
        assert!(class_hint_score("nav primary-nav") < 0.0);
        assert!(class_hint_score("share-buttons") < 0.0);
    }

    #[test]
    fn profile_for_listing_boosts_lists() {
        let p = profile_for(PageType::Listing);
        assert!((p.tag_weight)("ul") > (p.tag_weight)("p"));
    }

    #[test]
    fn profile_for_docs_boosts_pre() {
        let p = profile_for(PageType::Documentation);
        assert!((p.tag_weight)("pre") > (p.tag_weight)("p"));
    }

    #[test]
    fn profile_for_forum_reduces_link_density_penalty() {
        let base = profile_for(PageType::Article);
        let forum = profile_for(PageType::Forum);
        assert!(forum.w_link_density < base.w_link_density);
    }
}
