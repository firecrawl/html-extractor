//! Throughput benchmarks. See `TESTING.md` for the target buckets.
//!
//! Run with `cargo bench -p html-extractor`. The harness reads three sample
//! pages bundled at compile time so the bench is hermetic.

use criterion::{criterion_group, criterion_main, Criterion, Throughput};
use html_extractor::{extract, ExtractOptions};

const SMALL: &str = include_str!("../tests/fixtures/articles/simple_article.html");
const MEDIUM: &str = include_str!("../tests/fixtures/articles/news_typical.html");
const LARGE: &str = include_str!("../tests/fixtures/edge_cases/large_doc.html");

fn bench_throughput(c: &mut Criterion) {
    let opts = ExtractOptions::default();
    for (name, html) in [("small_10kb", SMALL), ("medium_200kb", MEDIUM), ("large_2mb", LARGE)] {
        let bytes = html.len() as u64;
        let mut group = c.benchmark_group("extract");
        group.throughput(Throughput::Bytes(bytes));
        group.bench_function(name, |b| {
            b.iter(|| {
                let r = extract(html, &opts).unwrap();
                criterion::black_box(r.markdown.len())
            })
        });
        group.finish();
    }
}

criterion_group!(benches, bench_throughput);
criterion_main!(benches);
