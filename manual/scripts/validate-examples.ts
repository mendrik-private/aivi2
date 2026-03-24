import fs from 'node:fs'
import path from 'node:path'
import { execSync } from 'node:child_process'
import os from 'node:os'

// ── Types ──────────────────────────────────────────────────────────────────

interface Snippet {
  sourcePath: string
  startLine:  number
  content:    string
}

interface ValidationError {
  sourcePath: string
  startLine:  number
  message:    string
}

// ── Helpers ────────────────────────────────────────────────────────────────

function aiviInPath(): boolean {
  try {
    execSync('which aivi', { stdio: 'ignore' })
    return true
  } catch {
    return false
  }
}

function globMarkdown(root: string): string[] {
  const results: string[] = []

  function walk(dir: string): void {
    for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
      const full = path.join(dir, entry.name)
      if (entry.isDirectory()) {
        if (entry.name === 'node_modules' || entry.name === 'dist') continue
        walk(full)
      } else if (entry.isFile() && entry.name.endsWith('.md')) {
        results.push(full)
      }
    }
  }

  walk(root)
  return results
}

function extractSnippets(filePath: string): Snippet[] {
  const lines   = fs.readFileSync(filePath, 'utf8').split('\n')
  const results: Snippet[] = []
  let   inside  = false
  let   start   = 0
  const buf: string[] = []

  for (let i = 0; i < lines.length; i++) {
    const line = lines[i]
    if (!inside && /^```aivi\s*$/.test(line)) {
      inside = true
      start  = i + 2  // 1-based line of first content line
      buf.length = 0
    } else if (inside && /^```\s*$/.test(line)) {
      inside = false
      results.push({ sourcePath: filePath, startLine: start, content: buf.join('\n') })
    } else if (inside) {
      buf.push(line)
    }
  }

  return results
}

// ── Main ───────────────────────────────────────────────────────────────────

if (!aiviInPath()) {
  console.warn('⚠  aivi not found in PATH — skipping validation')
  process.exit(0)
}

const manualRoot = path.resolve(import.meta.dirname ?? __dirname, '..')
const tmpDir     = path.join(os.tmpdir(), 'aivi-doc-check')

fs.mkdirSync(tmpDir, { recursive: true })

const markdownFiles = globMarkdown(manualRoot)
const errors: ValidationError[] = []
let   snippetIndex = 0

for (const mdFile of markdownFiles) {
  const snippets = extractSnippets(mdFile)
  for (const snippet of snippets) {
    const tmpFile = path.join(tmpDir, `snippet-${snippetIndex}.aivi`)
    fs.writeFileSync(tmpFile, snippet.content + '\n', 'utf8')

    const rel = path.relative(manualRoot, snippet.sourcePath)

    // aivi check
    try {
      execSync(`aivi check ${tmpFile}`, { stdio: 'pipe' })
    } catch (err: unknown) {
      const msg = (err as { stderr?: Buffer; stdout?: Buffer }).stderr?.toString()
             ?? (err as { stdout?: Buffer }).stdout?.toString()
             ?? String(err)
      for (const line of msg.trim().split('\n')) {
        errors.push({ sourcePath: rel, startLine: snippet.startLine, message: line })
      }
    }

    // aivi fmt --check
    try {
      execSync(`aivi fmt --check ${tmpFile}`, { stdio: 'pipe' })
    } catch (err: unknown) {
      const msg = (err as { stderr?: Buffer; stdout?: Buffer }).stderr?.toString()
             ?? (err as { stdout?: Buffer }).stdout?.toString()
             ?? 'formatting divergence'
      for (const line of msg.trim().split('\n')) {
        errors.push({ sourcePath: rel, startLine: snippet.startLine, message: `fmt: ${line}` })
      }
    }

    snippetIndex++
  }
}

if (errors.length > 0) {
  for (const e of errors) {
    console.error(`ERROR  ${e.sourcePath}:${e.startLine}: ${e.message}`)
  }
  process.exit(1)
}

console.log(`✓ Validated ${snippetIndex} AIVI code example(s) across ${markdownFiles.length} file(s).`)
process.exit(0)
