import { test } from 'node:test'
import assert from 'node:assert/strict'
import { extract, extractSync, version } from '../index.js'

const SIMPLE = `
  <html lang="en">
    <head>
      <title>Hello World</title>
      <meta name="author" content="A. Person">
      <meta property="og:site_name" content="ExampleSite">
    </head>
    <body>
      <header><nav><a href="/">Home</a></nav></header>
      <main>
        <article>
          <h1>Hello World</h1>
          <p>This is the first paragraph of an article. It is long enough to clear the extraction threshold and contains real prose that should be preserved in the markdown output.</p>
          <p>A second paragraph adds confidence to the scored walk and gives the extractor a solid candidate for the main content region of the page.</p>
        </article>
      </main>
      <footer>© 2024 ExampleSite</footer>
    </body>
  </html>
`

test('extract returns a Promise<ExtractResult> with expected fields', async () => {
  const r = await extract(SIMPLE)
  assert.ok(r.markdown.includes('Hello World'), `markdown should contain title, got: ${r.markdown}`)
  assert.ok(r.markdown.includes('first paragraph'))
  assert.ok(!r.markdown.includes('© 2024 ExampleSite'), 'footer should be dropped')
  assert.equal(typeof r.extractionQuality, 'number')
  assert.ok(r.extractionQuality > 0.15, `quality too low: ${r.extractionQuality}`)
  assert.equal(typeof r.pageType, 'string')
  assert.ok(r.metadata)
  assert.equal(r.metadata.author, 'A. Person')
  assert.equal(r.metadata.siteName, 'ExampleSite')
  assert.equal(r.metadata.language, 'en')
})

test('extractSync matches extract for the same input', async () => {
  const sync = extractSync(SIMPLE)
  const asyncResult = await extract(SIMPLE)
  assert.equal(sync.markdown, asyncResult.markdown)
  assert.equal(sync.pageType, asyncResult.pageType)
})

test('options marshal correctly — pageTypeOverride and outputText', async () => {
  const r = await extract(SIMPLE, { pageTypeOverride: 'documentation', outputText: true })
  assert.equal(r.pageType, 'documentation')
  assert.ok(r.text)
  assert.ok(!r.text.includes('#'), 'plain text should not contain markdown heading markers')
})

test('empty input returns errorReason without throwing', async () => {
  const r = await extract('')
  assert.equal(r.markdown, '')
  assert.equal(r.extractionQuality, 0)
  assert.ok(r.errorReason)
})

test('conflicting options reject the promise', async () => {
  await assert.rejects(
    () => extract(SIMPLE, { favorPrecision: true, favorRecall: true }),
    /conflicting options/
  )
})

test('version() returns a semver-shaped string', () => {
  const v = version()
  assert.match(v, /^\d+\.\d+\.\d+/)
})

test('async extract releases the event loop (concurrent calls work)', async () => {
  const N = 8
  const promises = []
  for (let i = 0; i < N; i++) {
    promises.push(extract(SIMPLE))
  }
  const results = await Promise.all(promises)
  for (const r of results) {
    assert.ok(r.markdown.includes('Hello World'))
  }
})
