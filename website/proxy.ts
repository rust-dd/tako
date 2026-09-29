import { isMarkdownPreferred, rewritePath } from 'fumadocs-core/negotiation';
import { type NextRequest, NextResponse } from 'next/server';

const docsMarkdown = rewritePath('/docs{/*path}', '/llms.mdx/docs{/*path}');

export default function proxy(request: NextRequest) {
  if (isMarkdownPreferred(request)) {
    const target = docsMarkdown.rewrite(request.nextUrl.pathname);
    if (target) return NextResponse.rewrite(new URL(target, request.nextUrl));
  }
  return NextResponse.next();
}

export const config = {
  matcher: ['/docs', '/docs/:path*'],
};
