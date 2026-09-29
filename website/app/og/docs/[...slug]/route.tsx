import { ImageResponse } from 'next/og';
import { notFound } from 'next/navigation';
import { OgImage, ogSize } from '@/components/OgImage';
import { pageImage, source } from '@/lib/source';

export const revalidate = false;

export async function GET(_req: Request, { params }: { params: Promise<{ slug: string[] }> }) {
  const { slug } = await params;
  const page = source.getPage(slug.slice(0, -1));
  if (!page) notFound();

  return new ImageResponse(
    <OgImage title={page.data.title} description={page.data.description} />,
    ogSize,
  );
}

export function generateStaticParams() {
  return source.getPages().map((page) => ({ slug: pageImage(page).segments }));
}
