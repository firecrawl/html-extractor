//! Smallest end-to-end example for the Rust API.
//!
//! Run with:
//!   cargo run --example extract_one -p html-extractor

use html_extractor::{extract, ExtractOptions};

fn main() {
    let html = r#"<!DOCTYPE html>
<html lang="en">
<head>
  <title>Hello — Example</title>
  <meta name="author" content="A. Person">
</head>
<body>
  <header><nav>navigation here</nav></header>
  <main>
    <article>
      <h1>Hello</h1>
      <p>This is the body of a small example article. It has enough characters and stop words to clear the scored walk and become the chosen main content.</p>
      <p>A second paragraph to push it well clear of the minimum extraction threshold and exercise the post-clean pass.</p>
    </article>
  </main>
  <footer>© 2024</footer>
</body>
</html>"#;

    let opts = ExtractOptions::default();
    let result = extract(html, &opts).expect("extract should succeed on a valid document");
    println!("--- markdown ---");
    println!("{}", result.markdown);
    println!();
    println!("page_type:          {:?}", result.page_type);
    println!("extraction_quality: {:.3}", result.extraction_quality);
    if let Some(md) = result.metadata {
        println!("title:              {:?}", md.title);
        println!("author:             {:?}", md.author);
    }
}
