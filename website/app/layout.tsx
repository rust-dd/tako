import './global.css';
import { RootProvider } from 'fumadocs-ui/provider/next';
import type { ReactNode } from 'react';
import type { Metadata } from 'next';
import { siteDescription, siteName, siteTitle, siteUrl } from '@/lib/site';

export const metadata: Metadata = {
  title: {
    default: siteTitle,
    template: '%s | Tako Rust web framework',
  },
  description: siteDescription,
  applicationName: siteName,
  metadataBase: new URL(siteUrl),
  openGraph: {
    type: 'website',
    siteName,
    locale: 'en_US',
    images: '/og/home.png',
  },
  twitter: {
    card: 'summary_large_image',
  },
};

export default function Layout({ children }: { children: ReactNode }) {
  return (
    <html lang="en" suppressHydrationWarning>
      <body className="flex flex-col min-h-screen">
        <RootProvider>{children}</RootProvider>
      </body>
    </html>
  );
}
