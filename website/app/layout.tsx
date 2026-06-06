import './global.css';
import { RootProvider } from 'fumadocs-ui/provider/next';
import type { ReactNode } from 'react';
import type { Metadata } from 'next';

export const metadata: Metadata = {
  title: {
    default: 'tako',
    template: '%s | tako',
  },
  description:
    'Multi-transport Rust framework for modern network services — HTTP/1.1, HTTP/2, HTTP/3, WebSocket, SSE, gRPC, TCP, UDP, Unix sockets, and WebTransport under one routing, middleware, and observability model.',
  metadataBase: new URL('https://tako.rust-dd.com'),
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
