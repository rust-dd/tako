import { llmsFull } from '@/lib/llm';

export const revalidate = false;

export async function GET() {
  return new Response(await llmsFull(), {
    headers: { 'Content-Type': 'text/plain; charset=utf-8' },
  });
}
