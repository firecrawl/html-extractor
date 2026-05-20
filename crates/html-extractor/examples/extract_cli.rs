//! Read an HTML file from argv[1] (or stdin if "-"), run extract(), emit JSON.
//! Internal — drives the comparison harness at /tmp/compare.py.
//!
//! Output schema (one JSON object on stdout, no pretty-print):
//!   {
//!     "markdown": "...",
//!     "page_type": "Article" | "Forum" | "Product" | ...,
//!     "extraction_quality": 0.0..1.0,
//!     "duration_us": <median of N runs>,
//!     "raw_bytes": <input length>
//!   }
//!
//! ENV:
//!   URL   — optional, passed as `ExtractOptions.url`
//!   RUNS  — how many times to run extract() and median over (default 5)

use html_extractor::{extract, ExtractOptions};
use std::env;
use std::io::Read;
use std::time::Instant;

fn read_input() -> std::io::Result<String> {
    let args: Vec<String> = env::args().collect();
    let target = args.get(1).cloned().unwrap_or_else(|| "-".to_string());
    if target == "-" {
        let mut buf = String::new();
        std::io::stdin().read_to_string(&mut buf)?;
        Ok(buf)
    } else {
        std::fs::read_to_string(target)
    }
}

fn main() {
    let html = read_input().expect("read input");
    let url = env::var("URL").ok();
    let runs: u32 = env::var("RUNS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(5);

    let opts = ExtractOptions {
        url,
        ..ExtractOptions::default()
    };

    let mut timings_us: Vec<u128> = Vec::with_capacity(runs as usize);
    let mut last = None;
    for _ in 0..runs {
        let start = Instant::now();
        let res = extract(&html, &opts).expect("extract");
        let elapsed = start.elapsed().as_micros();
        timings_us.push(elapsed);
        last = Some(res);
    }
    timings_us.sort_unstable();
    let median = timings_us[timings_us.len() / 2];

    let r = last.expect("at least one run");

    // Hand-rolled JSON to avoid a serde_json dep just for this example.
    let md_escaped = json_escape(&r.markdown);
    let page_type_dbg = format!("{:?}", r.page_type);
    let pt_escaped = json_escape(&page_type_dbg);
    let used_fallback = r.stats.as_ref().map(|s| s.used_fallback).unwrap_or(false);
    let text_chars = r.stats.as_ref().map(|s| s.text_chars).unwrap_or(0);
    let element_count = r.stats.as_ref().map(|s| s.element_count).unwrap_or(0);
    println!(
        r#"{{"markdown":"{}","page_type":"{}","extraction_quality":{:.4},"duration_us":{},"raw_bytes":{},"used_fallback":{},"text_chars":{},"element_count":{}}}"#,
        md_escaped,
        pt_escaped,
        r.extraction_quality,
        median,
        html.len(),
        used_fallback,
        text_chars,
        element_count,
    );
}

fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 16);
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\x08' => out.push_str("\\b"),
            '\x0c' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}
