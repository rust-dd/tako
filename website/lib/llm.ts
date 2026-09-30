import { readFile } from 'node:fs/promises';
import { join } from 'node:path';
import type { Node } from 'fumadocs-core/page-tree';
import { repoUrl, siteDescription, siteUrl } from '@/lib/site';
import { type DocsPageData, markdownUrl, source } from '@/lib/source';
import { crateMinorVersion, workspaceRoot } from '@/lib/workspace';

const rustExample = /<RustExample\s+path="([^"]+)"\s*\/>/g;

export function llmsHeader() {
  return [
    '# Tako',
    '',
    `> ${siteDescription}`,
    '',
    'Key facts for writing Tako code:',
    '',
    '- The crates.io package is `tako-rs`; the library is imported as `tako` (`use tako::...`). The crate named `tako` on crates.io is an unrelated project, so never depend on it.',
    `- Install with \`cargo add tako-rs\` (\`tako-rs = "${crateMinorVersion}"\`). Tako requires Rust 1.95 or newer and edition 2024.`,
    '- Default features give Tokio with HTTP/1.1, raw TCP, Unix sockets, static files, and the core extractors and middleware. HTTP/2, HTTP/3, TLS, WebSocket (`ws`), SSE (`sse`), UDP, gRPC, and the bundled plugins are opt-in Cargo features.',
    '- Start servers with `Server::builder().build().try_spawn_http(listener, router)?`; the `serve_*` free functions are deprecated. The `compio` feature switches to `CompioServer`; every transport runs on both runtimes.',
    '- Tako has no outbound HTTP client (`tako::client` was removed after 2.2.0); use `reqwest` or `ureq`.',
    '- Every docs page is available as Markdown by appending `.mdx` to its URL.',
  ].join('\n');
}

export function llmsIndex() {
  const lines = [llmsHeader(), '', '## Overview', ''];
  indexNodes(source.pageTree.children, 0, lines);
  lines.push(
    '',
    '## Optional',
    '',
    `- [Full documentation](${siteUrl}/llms-full.txt): every page above in one Markdown file`,
    '- [API reference](https://docs.rs/tako-rs/latest/tako/): rustdoc for the `tako` crate',
    `- [Examples](${repoUrl}/tree/main/examples): runnable example crates for every transport`,
    `- [Releases](${repoUrl}/releases): changelog and migration notes`,
  );
  return lines.join('\n');
}

export async function llmsFull() {
  const pages = await Promise.all(orderedPages().map(pageMarkdown));
  return [llmsHeader(), ...pages].join('\n\n---\n\n');
}

export async function pageMarkdown(page: DocsPageData) {
  const processed = await page.data.getText('processed');
  const body = await inlineRustExamples(processed.replace(/^\s*#\s+.*\n+/, ''));
  return [
    `# ${page.data.title}`,
    `> ${page.data.description}`,
    body.trim(),
    `Source: ${siteUrl}${page.url}`,
  ].join('\n\n');
}

function indexNodes(nodes: Node[], depth: number, lines: string[]) {
  const indent = '  '.repeat(depth);
  let section = '';
  for (const node of nodes) {
    if (node.type === 'separator') {
      section = nodeName(node.name);
      lines.push('', `## ${section}`, '');
    } else if (node.type === 'page') {
      const page = source.getNodePage(node);
      if (page) lines.push(`${indent}${indexLine(page)}`);
    } else {
      // A folder named like its section heading would only repeat that heading.
      const flatten = depth === 0 && nodeName(node.name) === section;
      if (!flatten) lines.push(`${indent}- ${nodeName(node.name)}`);
      const childDepth = flatten ? depth : depth + 1;
      const index = node.index && source.getNodePage(node.index);
      if (index) lines.push(`${'  '.repeat(childDepth)}${indexLine(index)}`);
      indexNodes(node.children, childDepth, lines);
    }
  }
}

function indexLine(page: DocsPageData) {
  return `- [${page.data.title}](${siteUrl}${markdownUrl(page)}): ${page.data.description}`;
}

function orderedPages() {
  const pages: DocsPageData[] = [];
  const visit = (nodes: Node[]) => {
    for (const node of nodes) {
      if (node.type === 'page') {
        const page = source.getNodePage(node);
        if (page) pages.push(page);
      } else if (node.type === 'folder') {
        const index = node.index && source.getNodePage(node.index);
        if (index) pages.push(index);
        visit(node.children);
      }
    }
  };
  visit(source.pageTree.children);
  return pages;
}

function nodeName(name: unknown) {
  return typeof name === 'string' ? name : '';
}

async function inlineRustExamples(markdown: string) {
  const paths = [...new Set([...markdown.matchAll(rustExample)].map((match) => match[1]))];
  const files = new Map(
    await Promise.all(
      paths.map(async (path) => [path, await readFile(join(workspaceRoot, path), 'utf8')] as const),
    ),
  );
  return markdown.replace(
    rustExample,
    (_, path: string) =>
      `Source: [${path}](${repoUrl}/blob/main/${path})\n\n\`\`\`rust\n${files.get(path)?.trimEnd()}\n\`\`\``,
  );
}
