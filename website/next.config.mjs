import { createMDX } from 'fumadocs-mdx/next';

const withMDX = createMDX();

/** @type {import('next').NextConfig} */
const config = {
  reactStrictMode: true,
  experimental: {
    optimizePackageImports: ['fumadocs-ui', 'fumadocs-core'],
  },
  async rewrites() {
    return [
      { source: '/docs.mdx', destination: '/llms.mdx/docs' },
      { source: '/docs/:path*.mdx', destination: '/llms.mdx/docs/:path*' },
    ];
  },
};

export default withMDX(config);
