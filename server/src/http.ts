/**
 * HTTP plumbing: JSON replies and bounded body reading.
 */
import { concat } from './bytes';

/** What a handler or Durable Object answers; serializable over RPC. */
export interface Reply {
  status: number;
  body: Record<string, unknown>;
}

const JSON_HEADERS = {
  'content-type': 'application/json; charset=utf-8',
  'cache-control': 'no-store',
  'x-content-type-options': 'nosniff',
} as const;

/** A JSON response. */
export function json(
  body: Record<string, unknown>,
  status = 200,
  extraHeaders: Record<string, string> = {},
): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { ...JSON_HEADERS, ...extraHeaders },
  });
}

/** A reply carrying a stable error code. */
export function failure(status: number, error: string): Reply {
  return { status, body: { error } };
}

/** Turns a reply into a response. */
export function respond(reply: Reply): Response {
  return json(reply.body, reply.status);
}

/** The whole body if it is at most `limit` bytes, else `null`. */
export async function readBody(request: Request, limit: number): Promise<Uint8Array | null> {
  const declared = request.headers.get('content-length');
  if (declared !== null && Number(declared) > limit) {
    return null;
  }
  if (request.body === null) {
    return new Uint8Array();
  }
  const reader = request.body.getReader();
  const chunks: Uint8Array[] = [];
  let size = 0;
  for (;;) {
    // Request bodies are untyped streams; check what actually arrives.
    const result: ReadableStreamReadResult<unknown> = await reader.read();
    if (result.done) {
      return concat(...chunks);
    }
    const chunk = result.value;
    if (!(chunk instanceof Uint8Array)) {
      await reader.cancel();
      return null;
    }
    size += chunk.byteLength;
    if (size > limit) {
      await reader.cancel();
      return null;
    }
    chunks.push(chunk);
  }
}

/** Parses a JSON object body; `null` if it is not one. */
export function parseObject(body: Uint8Array): Record<string, unknown> | null {
  try {
    const value: unknown = JSON.parse(new TextDecoder().decode(body));
    return typeof value === 'object' && value !== null && !Array.isArray(value)
      ? (value as Record<string, unknown>)
      : null;
  } catch {
    return null;
  }
}
