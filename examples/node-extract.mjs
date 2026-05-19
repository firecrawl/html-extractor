#!/usr/bin/env node
// Smallest end-to-end example: pass HTML in, get back markdown + metadata.
//
// Run from repo root:
//   node examples/node-extract.mjs
//
// Requires the binding to have been built once:
//   cd crates/html-extractor-napi && npm install && npm run build

import { extract } from '../crates/html-extractor-napi/index.js'

const html = `
<!DOCTYPE html>
<html lang="en">
  <head>
    <title>Coastal storm reopens harbor — Example News</title>
    <meta name="author" content="Pat Reporter">
    <meta property="og:site_name" content="Example News">
    <link rel="canonical" href="https://example.com/articles/coastal-storm">
  </head>
  <body>
    <header class="site-header">
      <nav><a href="/">Home</a> <a href="/world">World</a></nav>
    </header>
    <main>
      <article>
        <h1>Coastal storm reopens harbor</h1>
        <p class="byline">By Pat Reporter</p>
        <p>The seaside village of Aldermouth reopened its harbor on Wednesday after a week of high winds and torrential rain that had grounded fishing boats and closed the seafront promenade.</p>
        <p>Local officials described damage to the harbor wall as moderate, with several sections of the protective barrier needing replacement before the autumn storm season.</p>
      </article>
    </main>
    <aside class="related-stories"><h3>Related</h3><ul><li><a href="/x">other story</a></li></ul></aside>
    <footer>© 2024 Example News</footer>
  </body>
</html>`

const result = await extract(html, {
  url: 'https://example.com/articles/coastal-storm',
  outputText: false,
  includeLinks: true,
})

console.log('--- markdown ---')
console.log(result.markdown)
console.log()
console.log('--- metadata ---')
console.log(JSON.stringify(result.metadata, null, 2))
console.log()
console.log(`page_type: ${result.pageType}`)
console.log(`extraction_quality: ${result.extractionQuality.toFixed(3)}`)
console.log(`element_count: ${result.stats.elementCount}`)
console.log(`text_chars: ${result.stats.textChars}`)
