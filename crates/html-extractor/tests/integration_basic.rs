//! Integration-level smoke tests for the public API.

use html_extractor::{extract, ExtractError, ExtractOptions, PageType};

const SIMPLE_ARTICLE: &str = include_str!("fixtures/articles/simple_article.html");

#[test]
fn extracts_simple_article_body() {
    let r = extract(SIMPLE_ARTICLE, &ExtractOptions::default()).unwrap();
    assert!(
        r.markdown.contains("Coastal town reopens after long storm"),
        "title heading should be in markdown, got: {}",
        r.markdown
    );
    assert!(
        r.markdown.contains("Aldermouth reopened its harbor"),
        "body text should be present"
    );
    // chrome should be gone
    assert!(!r.markdown.contains("Other story one"), "related sidebar should be dropped");
    assert!(!r.markdown.contains("All rights reserved"), "footer should be dropped");
    assert!(r.extraction_quality > 0.3, "expected reasonable confidence, got {}", r.extraction_quality);
}

#[test]
fn extracts_metadata_from_meta_tags() {
    let r = extract(SIMPLE_ARTICLE, &ExtractOptions::default()).unwrap();
    let md = r.metadata.expect("metadata present");
    assert_eq!(md.author.as_deref(), Some("Test Writer"));
    assert_eq!(md.site_name.as_deref(), Some("Example News"));
    assert_eq!(md.published_date.as_deref(), Some("2024-09-12T08:30:00Z"));
    assert!(md.title.as_deref().unwrap().starts_with("Simple Article"));
    assert_eq!(
        md.canonical_url.as_deref(),
        Some("https://example.com/articles/simple-article")
    );
    assert_eq!(md.language.as_deref(), Some("en"));
}

#[test]
fn page_type_classification_uses_url_hint() {
    let opts = ExtractOptions {
        url: Some("https://example.com/blog/2024/coastal-town".to_string()),
        ..Default::default()
    };
    let r = extract(SIMPLE_ARTICLE, &opts).unwrap();
    assert_eq!(r.page_type, PageType::Article);
}

#[test]
fn empty_input_returns_low_quality_with_reason() {
    let r = extract("", &ExtractOptions::default()).unwrap();
    assert!(r.markdown.is_empty());
    assert_eq!(r.extraction_quality, 0.0);
    assert_eq!(r.error_reason, Some(ExtractError::EmptyInput));
}

#[test]
fn conflicting_options_rejected() {
    let opts = ExtractOptions {
        favor_precision: true,
        favor_recall: true,
        ..Default::default()
    };
    let err = extract(SIMPLE_ARTICLE, &opts).unwrap_err();
    assert!(matches!(err, ExtractError::ConflictingOptions(_)));
}

#[test]
fn page_type_override_is_respected() {
    let opts = ExtractOptions {
        page_type_override: Some(PageType::Documentation),
        ..Default::default()
    };
    let r = extract(SIMPLE_ARTICLE, &opts).unwrap();
    assert_eq!(r.page_type, PageType::Documentation);
}

#[test]
fn no_panic_on_malformed_input() {
    // unclosed tags, weird structure
    let html = "<html><body><div><p>hello world this is some text that is long enough to maybe make it past extraction <span>nested<div>still here";
    let _ = extract(html, &ExtractOptions::default()).unwrap();
}

#[test]
fn page_with_no_semantic_markers_uses_scored_walk() {
    let html = r#"
        <html><body>
        <div class="header">Site nav <a href='/'>home</a> <a href='/x'>x</a></div>
        <div class="wrapper">
            <div class="content-body">
                <p>This is the first paragraph of the main content. It has multiple sentences and reads like real prose with enough length to score well. The author keeps writing.</p>
                <p>This is the second paragraph of the main content. It also reads like real prose with sufficient stop words and enough characters to push the score even higher.</p>
                <p>This is the third paragraph to push it past the fallback threshold so the scored walk picks this region.</p>
            </div>
        </div>
        <div class="footer">copyright 2024</div>
        </body></html>
    "#;
    let r = extract(html, &ExtractOptions::default()).unwrap();
    assert!(r.markdown.contains("first paragraph"), "got: {}", r.markdown);
    assert!(!r.markdown.contains("copyright 2024"));
}
