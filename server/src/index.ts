/**
 * Zelamar delivery service.
 *
 * Phase 0 skeleton: only a health endpoint. The real responsibilities
 * (username registry, device mailboxes, KeyPackages, sealed-sender delivery)
 * arrive in Phase 3. By design this service never sees plaintext.
 */
import packageJson from '../package.json';

const SERVICE_NAME = 'zelamar';

const JSON_HEADERS = {
  'content-type': 'application/json; charset=utf-8',
  'cache-control': 'no-store',
  'x-content-type-options': 'nosniff',
} as const;

function json(body: unknown, status = 200, extraHeaders: Record<string, string> = {}): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { ...JSON_HEADERS, ...extraHeaders },
  });
}

function handleHealth(request: Request): Response {
  if (request.method !== 'GET' && request.method !== 'HEAD') {
    return json({ error: 'method_not_allowed' }, 405, { allow: 'GET, HEAD' });
  }
  return json({ status: 'ok', service: SERVICE_NAME, version: packageJson.version });
}

export default {
  fetch(request): Response {
    const { pathname } = new URL(request.url);
    if (pathname === '/health') {
      return handleHealth(request);
    }
    return json({ error: 'not_found' }, 404);
  },
} satisfies ExportedHandler<Env>;
