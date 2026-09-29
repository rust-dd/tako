import { type DocsPageData, markdownUrl, pageImage, source } from '@/lib/source';
import { repoUrl, siteName, siteUrl } from '@/lib/site';
import {
  DocsPage,
  DocsBody,
  DocsDescription,
  DocsTitle,
} from 'fumadocs-ui/page';
import { MarkdownCopyButton, ViewOptionsPopover } from 'fumadocs-ui/layouts/docs/page';
import { notFound } from 'next/navigation';
import { getMDXComponents } from '@/mdx-components';
import { JsonLd } from '@/components/JsonLd';
import type { Metadata } from 'next';

export default async function Page(props: {
  params: Promise<{ slug?: string[] }>;
}) {
  const params = await props.params;
  const page = source.getPage(params.slug);
  if (!page) notFound();

  const MDX = page.data.body;
  const markdown = markdownUrl(page);

  return (
    <DocsPage toc={page.data.toc} full={page.data.full} lastUpdate={page.data.lastModified}>
      <JsonLd data={breadcrumbList(page)} />
      <DocsTitle>{page.data.title}</DocsTitle>
      <DocsDescription>{page.data.description}</DocsDescription>
      <div className="flex flex-row flex-wrap items-center gap-2 border-b pb-6">
        <MarkdownCopyButton markdownUrl={markdown} />
        <ViewOptionsPopover
          markdownUrl={markdown}
          githubUrl={`${repoUrl}/blob/main/website/content/docs/${page.path}`}
        />
      </div>
      <DocsBody>
        {/* The MDX H1 stays in the source for the Markdown export; DocsTitle is the page heading. */}
        <MDX components={getMDXComponents({ h1: () => null })} />
      </DocsBody>
    </DocsPage>
  );
}

function breadcrumbList(page: DocsPageData) {
  const trail = [{ name: 'Documentation', url: '/docs' }];
  for (let depth = 1; depth < page.slugs.length; depth++) {
    const parent = source.getPage(page.slugs.slice(0, depth));
    if (parent) trail.push({ name: parent.data.title, url: parent.url });
  }
  if (page.slugs.length > 0) trail.push({ name: page.data.title, url: page.url });

  return {
    '@context': 'https://schema.org',
    '@type': 'BreadcrumbList',
    itemListElement: trail.map((item, i) => ({
      '@type': 'ListItem',
      position: i + 1,
      name: item.name,
      item: `${siteUrl}${item.url}`,
    })),
  };
}

export async function generateStaticParams() {
  return source.generateParams();
}

export async function generateMetadata(props: {
  params: Promise<{ slug?: string[] }>;
}): Promise<Metadata> {
  const params = await props.params;
  const page = source.getPage(params.slug);
  if (!page) notFound();

  const image = pageImage(page).url;

  return {
    title: page.data.title,
    description: page.data.description,
    alternates: {
      canonical: page.url,
      types: { 'text/markdown': markdownUrl(page) },
    },
    openGraph: {
      type: 'article',
      siteName,
      locale: 'en_US',
      url: page.url,
      title: page.data.title,
      description: page.data.description,
      images: image,
    },
    twitter: {
      card: 'summary_large_image',
      images: image,
    },
  };
}
