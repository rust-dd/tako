import { defineConfig, defineDocs, frontmatterSchema } from 'fumadocs-mdx/config';
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
  },
});

export default defineConfig();
