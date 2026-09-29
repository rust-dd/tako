import { ImageResponse } from 'next/og';
import { OgImage, ogSize } from '@/components/OgImage';

export const revalidate = false;

export function GET() {
  return new ImageResponse(
    <OgImage
      title="Tako"
      description="One router for HTTP/1.1, HTTP/2, HTTP/3, WebSocket, SSE, gRPC, TCP, UDP and Unix sockets, on Tokio or Compio."
    />,
    ogSize,
  );
}
