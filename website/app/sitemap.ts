import type { MetadataRoute } from 'next';
import { siteUrl } from '@/lib/site';
import { source } from '@/lib/source';

export const revalidate = false;

export default function sitemap(): MetadataRoute.Sitemap {
  return [
    { url: siteUrl, changeFrequency: 'monthly', priority: 1 },
    ...source.getPages().map((page) => ({
      url: `${siteUrl}${page.url}`,
      lastModified: page.data.lastModified,
      changeFrequency: 'monthly' as const,
      priority: page.slugs.length === 0 ? 0.9 : 0.7,
    })),
  ];
}
