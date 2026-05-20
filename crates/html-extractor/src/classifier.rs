//! Stage 2 — page-type classifier.
//!
//! Cheap rules ladder per OPTIMIZATIONS.md #2: URL patterns, tag counts,
//! class hints, JSON-LD `@type`. Returns `(PageType, confidence)`.

use once_cell::sync::Lazy;
use regex::Regex;

use crate::tree::Tree;
use crate::types::{Metadata, PageType};

static URL_ARTICLE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)/(article|news|story|stories|blog|posts?|opinion|features?|columns?)(/|$)")
        .unwrap()
});
// NOTE: `topic` was previously here but it's ambiguous — many docs sites use
// `/topics/` as a section path (e.g. Django's `/en/5.0/topics/db/queries/`),
// so matching it as a forum signal mis-classifies docs as forums. Threads,
// discussions, and questions are unambiguous forum signals.
static URL_FORUM: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)/(forum|thread|discussion|question)s?(/|$)").unwrap());
static URL_PRODUCT: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)/(product|item|sku|p|dp|gp/product)/").unwrap());
static URL_LISTING: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)/(category|categories|catalog|search|listing|browse|shop|tag|tags)(/|$|\?)")
        .unwrap()
});
static URL_DOCS: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)/(docs?|documentation|reference|api|manual|guide|tutorial|faq)(/|$)").unwrap()
});
static URL_SERVICE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?i)/(pricing|plans?|contact|about|terms|privacy|legal|cookies?|imprint|support)(/|$)",
    )
    .unwrap()
});

static CLASS_FORUM: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)(\b|_|-)(thread|forum|post-(list|item)|comment-list|qa-message)\b").unwrap()
});
static CLASS_PRODUCT: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)(\b|_|-)(product(-detail|-info|-page)?|item-detail|sku-)\b").unwrap()
});
static CLASS_LISTING: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)(\b|_|-)(product-list|search-?results?|listing|cards?|product-grid|catalog)\b")
        .unwrap()
});
static CLASS_DOCS: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)(\b|_|-)(docs?(-content|-body)?|api-reference|sphinx)\b").unwrap()
});

/// Classify a parsed document.
pub(crate) fn classify(tree: &Tree, url: Option<&str>, metadata: &Metadata) -> (PageType, f32) {
    // Tally signals across sources. Each match contributes a small score; the
    // page type with the highest score wins. Confidence is `winner_score /
    // total_score`, floor 0.1.
    //
    // URL signals weighted at 5.0 — strong because URL patterns like /pricing,
    // /docs, /products are deliberate routing decisions by the site author
    // and historically reliable. They need to be strong enough to overpower
    // BOTH (a) repeated class hits and (b) structural tag-count bonuses (e.g.
    // a /pricing page with many "what's included" <li> bullets shouldn't get
    // pulled into Listing by the >20-li bonus). Class signals are accumulated
    // per category and then applied with harmonic (sub-linear) scaling so a
    // product-card grid on a /pricing page (e.g. Stripe's product nav) can't
    // drown out the URL_SERVICE signal: 1 match → +1.0, 2 → +1.5, 5 → +2.28,
    // 20 → +3.6.
    let mut scores = [0.0f32; 8];
    let mut class_counts = [0u32; 8];

    if let Some(u) = url {
        if URL_ARTICLE.is_match(u) {
            scores[PageType::Article as usize] += 5.0;
        }
        if URL_FORUM.is_match(u) {
            scores[PageType::Forum as usize] += 5.0;
        }
        if URL_PRODUCT.is_match(u) {
            scores[PageType::Product as usize] += 5.0;
        }
        if URL_LISTING.is_match(u) {
            scores[PageType::Listing as usize] += 5.0;
        }
        if URL_DOCS.is_match(u) {
            scores[PageType::Documentation as usize] += 5.0;
        }
        if URL_SERVICE.is_match(u) {
            scores[PageType::Service as usize] += 5.0;
        }
    }

    // JSON-LD @type signals (collected by metadata module into Metadata's
    // pagetype-like signal; for now we infer from description/site heuristics
    // and structural tag counts).

    // Structural counts: cheap pre-order walk over the original tree.
    let mut tag_counts = TagCounts::default();
    tree.walk_pre(tree.root, |idx| {
        let elem = tree.get(idx);
        match elem.tag.as_str() {
            "article" => tag_counts.article += 1,
            "h1" => tag_counts.h1 += 1,
            "h2" => tag_counts.h2 += 1,
            "table" => tag_counts.table += 1,
            "form" => tag_counts.form += 1,
            "main" => tag_counts.main += 1,
            "ul" | "ol" => tag_counts.list += 1,
            "li" => tag_counts.li += 1,
            "pre" | "code" => tag_counts.code += 1,
            _ => {}
        }
        // Class signals — counted here, scored below with harmonic scaling.
        let needle = elem.class_id_lower();
        if !needle.is_empty() {
            if CLASS_FORUM.is_match(&needle) {
                class_counts[PageType::Forum as usize] += 1;
            }
            if CLASS_PRODUCT.is_match(&needle) {
                class_counts[PageType::Product as usize] += 1;
            }
            if CLASS_LISTING.is_match(&needle) {
                class_counts[PageType::Listing as usize] += 1;
            }
            if CLASS_DOCS.is_match(&needle) {
                class_counts[PageType::Documentation as usize] += 1;
            }
        }
        true
    });

    // Apply harmonic-scaled class scores. Per-category weight matches the
    // previous code (CLASS_LISTING/DOCS at 0.7, others at 1.0).
    scores[PageType::Forum as usize] += harmonic_score(class_counts[PageType::Forum as usize], 1.0);
    scores[PageType::Product as usize] +=
        harmonic_score(class_counts[PageType::Product as usize], 1.0);
    scores[PageType::Listing as usize] +=
        harmonic_score(class_counts[PageType::Listing as usize], 0.7);
    scores[PageType::Documentation as usize] +=
        harmonic_score(class_counts[PageType::Documentation as usize], 0.7);

    if tag_counts.article >= 1 || tag_counts.main >= 1 {
        scores[PageType::Article as usize] += 1.5;
    }
    if tag_counts.li > 20 && tag_counts.article == 0 {
        scores[PageType::Listing as usize] += 2.0;
    }
    if tag_counts.code > 5 || tag_counts.h2 > 8 {
        scores[PageType::Documentation as usize] += 1.0;
    }
    if tag_counts.h1 > 4 {
        // Multiple H1s look more like a homepage than a single article.
        scores[PageType::Collection as usize] += 0.7;
    }

    // Metadata hints
    if let Some(t) = metadata.title.as_deref() {
        let lt = t.to_lowercase();
        if lt.contains("buy") || lt.contains(" $") || lt.contains("price") {
            scores[PageType::Product as usize] += 0.5;
        }
    }

    let total: f32 = scores.iter().sum();
    if total < 0.5 {
        return (PageType::Other, 0.0);
    }
    let (best_idx, best_score) = scores
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
        .map(|(i, s)| (i, *s))
        .unwrap();
    let conf = (best_score / total).clamp(0.1, 1.0);
    let pt = match best_idx {
        0 => PageType::Article,
        1 => PageType::Forum,
        2 => PageType::Product,
        3 => PageType::Listing,
        4 => PageType::Collection,
        5 => PageType::Documentation,
        6 => PageType::Service,
        _ => PageType::Other,
    };
    if conf < 0.35 {
        (PageType::Other, conf)
    } else {
        (pt, conf)
    }
}

/// Sub-linear (harmonic) scaling for repeated class hits: 1.0, 1.5, 1.83, 2.08,
/// ... The first match is full-weight evidence; each repeat adds less. Stops
/// repeated component-class hits (e.g. a 20-card product grid) from drowning
/// stronger signals like URL patterns.
fn harmonic_score(count: u32, weight: f32) -> f32 {
    if count == 0 {
        return 0.0;
    }
    let h: f32 = (1..=count).map(|n| 1.0 / n as f32).sum();
    h * weight
}

#[derive(Default)]
struct TagCounts {
    article: usize,
    h1: usize,
    h2: usize,
    table: usize,
    form: usize,
    main: usize,
    list: usize,
    li: usize,
    code: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse;
    use crate::types::Metadata;

    fn classify_html(html: &str, url: Option<&str>) -> (PageType, f32) {
        let tree = parse(html).unwrap();
        classify(&tree, url, &Metadata::default())
    }

    #[test]
    fn url_with_article_path_classifies_as_article() {
        let html = "<html><body><article><h1>x</h1><p>hello world</p></article></body></html>";
        let (pt, _) = classify_html(html, Some("https://example.com/news/2024/foo"));
        assert_eq!(pt, PageType::Article);
    }

    #[test]
    fn product_url_overrides_article_tag() {
        let html = "<html><body><div class='product-detail'><h1>Widget</h1></div></body></html>";
        let (pt, _) = classify_html(html, Some("https://shop.example.com/products/sku-1234"));
        assert_eq!(pt, PageType::Product);
    }

    #[test]
    fn docs_url_classifies_as_documentation() {
        let html =
            "<html><body><main><h2>API</h2><pre><code>fn x() {}</code></pre></main></body></html>";
        let (pt, _) = classify_html(html, Some("https://example.com/docs/api/x"));
        assert_eq!(pt, PageType::Documentation);
    }

    #[test]
    fn no_signals_returns_other() {
        let html = "<html><body><div>hello</div></body></html>";
        let (pt, _) = classify_html(html, None);
        assert_eq!(pt, PageType::Other);
    }

    #[test]
    fn listing_inferred_from_many_li() {
        let mut html = String::from("<html><body><ul>");
        for i in 0..60 {
            html.push_str(&format!("<li>item {i}</li>"));
        }
        html.push_str("</ul></body></html>");
        let (pt, _) = classify_html(&html, None);
        assert_eq!(pt, PageType::Listing);
    }

    #[test]
    fn service_url_beats_repeated_product_class_noise() {
        // Stripe-pricing-style failure mode: URL /pricing + many product-card
        // nodes from a product-nav grid. Pre-fix, 20× product class hits
        // accumulated to +20.0 and overwhelmed the URL_SERVICE +3.0 signal;
        // page classified as Product (wrong) → wrong scoring profile → main-
        // content extraction picked the product nav instead of the prices.
        let mut html = String::from("<html><body>");
        for _ in 0..20 {
            html.push_str(r#"<div class="product-detail">Payments product</div>"#);
        }
        html.push_str("</body></html>");
        let (pt, _) = classify_html(&html, Some("https://example.com/pricing"));
        assert_eq!(pt, PageType::Service);
    }

    #[test]
    fn harmonic_class_scaling() {
        // 1 hit = full weight, 2 hits = 1.5×, decay thereafter.
        assert!((harmonic_score(0, 1.0) - 0.0).abs() < 1e-6);
        assert!((harmonic_score(1, 1.0) - 1.0).abs() < 1e-6);
        assert!((harmonic_score(2, 1.0) - 1.5).abs() < 1e-6);
        // 20 hits with weight 1.0 lands around 3.6 — stays below the
        // URL_*-signal weight of 4.0 so URL can still win when present.
        let twenty = harmonic_score(20, 1.0);
        assert!(twenty > 3.5 && twenty < 4.0);
    }
}
