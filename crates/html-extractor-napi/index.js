// Minimal native-module loader. The napi-cli emits the platform-specific
// `.node` file; we resolve it here for the current platform/arch combo.
const { existsSync, readFileSync } = require('fs')
const { join } = require('path')

const { platform, arch } = process

function libcFlavor() {
  // Detect glibc vs musl on Linux by checking ldd output of /proc/self/exe.
  if (platform !== 'linux') return ''
  const reportPath = '/proc/self/exe'
  try {
    const fs = require('fs')
    const ldd = fs.readlinkSync(reportPath)
    if (ldd && ldd.includes('musl')) return '-musl'
  } catch (_) { /* ignore */ }
  return '-gnu'
}

const triples = {
  'darwin:arm64': 'html-extractor.darwin-arm64.node',
  'darwin:x64':   'html-extractor.darwin-x64.node',
  'linux:x64':    `html-extractor.linux-x64${libcFlavor()}.node`,
  'linux:arm64':  `html-extractor.linux-arm64${libcFlavor()}.node`,
  'win32:x64':    'html-extractor.win32-x64-msvc.node',
}

const key = `${platform}:${arch}`
const filename = triples[key]
if (!filename) {
  throw new Error(`html-extractor: unsupported platform ${key}`)
}
const candidate = join(__dirname, filename)
if (!existsSync(candidate)) {
  throw new Error(
    `html-extractor: prebuilt binary not found at ${candidate}.\n` +
    `Run \`npm run build\` in the binding crate to build for this platform.`
  )
}
const native = require(candidate)

module.exports.extract = native.extract
module.exports.extractSync = native.extractSync
module.exports.version = native.version
