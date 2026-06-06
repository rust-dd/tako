#!/usr/bin/env bun
/**
 * Full docs audit. Run in CI and before a release.
 *
 * Hard failures (exit 1):
 *   1. Frontmatter zod schema violation (delegates to lint-mdx.ts)
 *   2. MDX file missing from its directory's meta.json `pages` array
 *   3. <RustExample path="..." /> points at a file that does not exist
 *   4. Internal /docs/... link that does not resolve to a real page
 *
 * The link resolver builds the set of valid page paths from the content
 * tree and checks every markdown / href link that starts with `/docs`.
 * External links and in-page `#anchors` are not checked.
 */
import { spawnSync } from 'node:child_process';
import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join, relative } from 'node:path';

const ROOT = join(import.meta.dir, '..', 'content', 'docs');
const WORKSPACE = join(import.meta.dir, '..', '..');

let errors = 0;

function* walk(dir: string): Generator<string> {
  for (const entry of readdirSync(dir)) {
    const full = join(dir, entry);
    if (statSync(full).isDirectory()) yield* walk(full);
    else if (full.endsWith('.mdx')) yield full;
  }
}

function docPath(file: string): string {
  let rel = relative(ROOT, file).replace(/\.mdx$/, '').replace(/\\/g, '/');
  if (rel === 'index') return '/docs';
  if (rel.endsWith('/index')) rel = rel.slice(0, -'/index'.length);
  return `/docs/${rel}`;
}

{
  const r = spawnSync('bun', ['run', 'scripts/lint-mdx.ts'], {
    cwd: join(import.meta.dir, '..'),
    stdio: 'inherit',
  });
  if (r.status !== 0) errors++;
}

function checkMetaCoverage(dir: string) {
  const entries = readdirSync(dir);
  const metaPath = join(dir, 'meta.json');
  const mdxFiles = entries
    .filter((e) => e.endsWith('.mdx') && e !== 'index.mdx')
    .map((e) => e.replace(/\.mdx$/, ''));

  if (mdxFiles.length > 0) {
    let pages: string[] = [];
    try {
      const raw = JSON.parse(readFileSync(metaPath, 'utf8'));
      pages = (raw.pages ?? []) as string[];
    } catch {
      errors++;
      console.error(`✘ ${relative(WORKSPACE, dir)} missing or invalid meta.json`);
    }

    const declared = new Set(pages.filter((p) => !p.startsWith('---')));
    for (const slug of mdxFiles) {
      if (!declared.has(slug)) {
        errors++;
        console.error(
          `✘ ${relative(WORKSPACE, join(dir, slug + '.mdx'))} not in meta.json`,
        );
      }
    }
  }

  for (const e of entries) {
    const full = join(dir, e);
    if (statSync(full).isDirectory()) checkMetaCoverage(full);
  }
}

checkMetaCoverage(ROOT);

const validPaths = new Set<string>();
for (const file of walk(ROOT)) validPaths.add(docPath(file));

const rustExampleRe = /<RustExample\s+path="([^"]+)"\s*\/>/g;
const linkRe = /\]\((\/docs[^)\s]*)\)|href="(\/docs[^"]*)"/g;

for (const file of walk(ROOT)) {
  const rel = relative(WORKSPACE, file);
  const src = readFileSync(file, 'utf8');

  let m: RegExpExecArray | null;
  while ((m = rustExampleRe.exec(src)) !== null) {
    const target = join(WORKSPACE, m[1]);
    let isFile = false;
    try {
      isFile = statSync(target).isFile();
    } catch {
      isFile = false;
    }
    if (!isFile) {
      errors++;
      console.error(`✘ ${rel}: RustExample path "${m[1]}" is not a file`);
    }
  }
  rustExampleRe.lastIndex = 0;

  while ((m = linkRe.exec(src)) !== null) {
    const raw = m[1] ?? m[2];
    const path = raw.replace(/#.*$/, '').replace(/\/$/, '') || '/docs';
    if (!validPaths.has(path)) {
      errors++;
      console.error(`✘ ${rel}: broken internal link "${raw}"`);
    }
  }
  linkRe.lastIndex = 0;
}

if (errors > 0) {
  console.error(`\n${errors} error(s).`);
  process.exit(1);
}

console.log('✔ docs audit clean (frontmatter, meta.json coverage, RustExample paths, internal links).');
