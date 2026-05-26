//! Golden corpus runner.
//!
//! Walks `tests/fixtures/<category>/` for every `<name>.html` that has a
//! matching `<name>.meta.json`. For each fixture asserts:
//!
//!   - every line of `<name>.expected.md` appears (as a substring) in the
//!     extractor's markdown output;
//!   - none of `meta.must_not_contain` appears;
//!   - if `meta.page_type` is set, the detected page_type matches;
//!   - `extraction_quality >= meta.min_quality`;
//!   - the public API never panics for any fixture.
//!
//! Golden assertions are permissive on whitespace and exact tokens because
//! we want them to gate on "did the article body end up in the output, did
//! the chrome leave" — not on micro-formatting that's fine to evolve.

use html_extractor::{extract, ExtractOptions, PageType};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, serde::Deserialize, Default)]
struct GoldenMeta {
    #[serde(default)]
    page_type: Option<String>,
    /// Source URL of the page. Passed to the classifier just like production
    /// does — URL is the primary signal for product / docs / listing / service
    /// page types. Article / forum / collection classify structurally and may
    /// omit it.
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    author: Option<String>,
    #[serde(default)]
    language: Option<String>,
    #[serde(default)]
    min_quality: f32,
    #[serde(default)]
    must_not_contain: Vec<String>,
}

const FIXTURE_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures");

fn parse_page_type(s: &str) -> PageType {
    match s {
        "article" => PageType::Article,
        "forum" => PageType::Forum,
        "product" => PageType::Product,
        "listing" => PageType::Listing,
        "collection" => PageType::Collection,
        "documentation" => PageType::Documentation,
        "service" => PageType::Service,
        _ => PageType::Other,
    }
}

fn enumerate_fixtures() -> Vec<PathBuf> {
    let mut out = Vec::new();
    let root = Path::new(FIXTURE_DIR);
    for entry in fs::read_dir(root).unwrap() {
        let category = entry.unwrap();
        if !category.path().is_dir() {
            continue;
        }
        for f in fs::read_dir(category.path()).unwrap() {
            let p = f.unwrap().path();
            if p.extension().and_then(|s| s.to_str()) == Some("html") {
                let meta = p.with_extension("meta.json");
                if meta.exists() {
                    out.push(p);
                }
            }
        }
    }
    out.sort();
    out
}

#[test]
fn golden_corpus() {
    let fixtures = enumerate_fixtures();
    assert!(
        fixtures.len() >= 50,
        "golden corpus must have ≥50 fixtures (have {})",
        fixtures.len()
    );

    let mut failures: Vec<String> = Vec::new();
    for path in &fixtures {
        let html = fs::read_to_string(path).expect("read html");
        let meta_path = path.with_extension("meta.json");
        let expected_path = path.with_extension("expected.md");
        let meta: GoldenMeta =
            serde_json::from_str(&fs::read_to_string(&meta_path).expect("read meta"))
                .expect("parse meta");
        let expected = fs::read_to_string(&expected_path).unwrap_or_default();

        // Pass the source URL when the fixture carries one, exactly as the
        // production caller does. We never set page_type_override — the point
        // is to exercise the real classifier. include_metadata defaults to true.
        let opts = ExtractOptions {
            url: meta.url.clone(),
            ..Default::default()
        };
        let r = match extract(&html, &opts) {
            Ok(r) => r,
            Err(e) => {
                failures.push(format!("{}: extract returned Err({e:?})", path.display()));
                continue;
            }
        };

        let fixture = path
            .strip_prefix(FIXTURE_DIR)
            .unwrap()
            .display()
            .to_string();
        for needle in expected.lines().filter(|l| !l.trim().is_empty()) {
            let needle = needle.trim();
            if !r.markdown.contains(needle) {
                failures.push(format!(
                    "{fixture}: expected substring not found: {needle:?}\n--- markdown ---\n{}\n---",
                    truncate(&r.markdown, 600)
                ));
            }
        }
        for bad in &meta.must_not_contain {
            if r.markdown.contains(bad) {
                failures.push(format!(
                    "{fixture}: forbidden substring present: {bad:?}\n--- markdown ---\n{}\n---",
                    truncate(&r.markdown, 600)
                ));
            }
        }
        if r.extraction_quality + 1e-6 < meta.min_quality {
            failures.push(format!(
                "{fixture}: extraction_quality {:.3} below threshold {:.3}",
                r.extraction_quality, meta.min_quality
            ));
        }
        if let Some(pt) = meta.page_type.as_deref() {
            let expected_pt = parse_page_type(pt);
            if r.page_type != expected_pt {
                failures.push(format!(
                    "{fixture}: page_type mismatch — expected {expected_pt:?}, got {:?}",
                    r.page_type
                ));
            }
        }
        if let Some(meta_title) = meta.title.as_deref() {
            if let Some(actual) = r.metadata.as_ref().and_then(|m| m.title.clone()) {
                if !actual.contains(meta_title) {
                    failures.push(format!(
                        "{fixture}: metadata title mismatch — expected to contain {meta_title:?}, got {actual:?}",
                    ));
                }
            }
        }
        if let Some(expected_author) = meta.author.as_deref() {
            let actual = r.metadata.as_ref().and_then(|m| m.author.clone());
            if actual.as_deref() != Some(expected_author) {
                failures.push(format!(
                    "{fixture}: metadata author mismatch — expected {expected_author:?}, got {actual:?}",
                ));
            }
        }
        if let Some(expected_lang) = meta.language.as_deref() {
            let actual = r.metadata.as_ref().and_then(|m| m.language.clone());
            if actual.as_deref() != Some(expected_lang) {
                failures.push(format!(
                    "{fixture}: metadata language mismatch — expected {expected_lang:?}, got {actual:?}",
                ));
            }
        }
    }

    if !failures.is_empty() {
        for f in &failures {
            eprintln!("FAIL {f}");
        }
        panic!(
            "{} golden-corpus failures (of {} fixtures)",
            failures.len(),
            fixtures.len()
        );
    }

    eprintln!("golden corpus: {} fixtures passed", fixtures.len());
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let mut out = String::new();
        for (i, c) in s.chars().enumerate() {
            if i >= n {
                break;
            }
            out.push(c);
        }
        out.push('…');
        out
    }
}
