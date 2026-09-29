import { docs } from '@/.source/server';
import { type InferPageType, loader } from 'fumadocs-core/source';

export const source = loader({
  baseUrl: '/docs',
  source: docs.toFumadocsSource(),
});

export type DocsPageData = InferPageType<typeof source>;

export function markdownUrl(page: DocsPageData) {
  return `${page.url}.mdx`;
}

export function pageImage(page: DocsPageData) {
  const segments = [...page.slugs, 'image.png'];
  return { segments, url: `/og/docs/${segments.join('/')}` };
}
