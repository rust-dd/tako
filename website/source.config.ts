import { execFileSync } from 'node:child_process';
import { defineConfig, defineDocs, frontmatterSchema } from 'fumadocs-mdx/config';
import lastModified from 'fumadocs-mdx/plugins/last-modified';
import { z } from 'zod';

const takoFrontmatter = frontmatterSchema.extend({
  category: z
    .enum([
      'concept',
      'guide',
      'transport',
      'extractor',
      'middleware',
      'plugin',
      'tutorial',
      'reference',
    ])
    .optional(),
  subcategory: z.string().optional(),
  crate: z
    .string()
    .regex(/^tako-rs(-[a-z]+)*$/)
    .optional(),
  module_path: z.string().optional(),
  since: z
    .string()
    .regex(/^\d+\.\d+(\.\d+)?(-[a-z0-9.]+)?$/)
    .optional(),
  status: z.enum(['stable', 'experimental', 'deprecated']).optional(),
  runtime: z.enum(['tokio', 'compio', 'both']).optional(),
  features: z.array(z.string()).default([]),
  replaced_by: z.string().optional(),
});

export const docs = defineDocs({
  dir: 'content/docs',
  docs: {
    schema: takoFrontmatter,
    postprocess: {
      includeProcessedMarkdown: { headingIds: false },
    },
  },
});

// A shallow clone (the Vercel default) dates every file to the clone
// boundary, so pages only show a last-updated date from full history.
function isShallowClone() {
  try {
    return (
      execFileSync('git', ['rev-parse', '--is-shallow-repository'], { encoding: 'utf8' }).trim() ===
      'true'
    );
  } catch {
    return true;
  }
}

export default defineConfig({
  plugins: [lastModified(isShallowClone() ? { versionControl: async () => null } : {})],
});
