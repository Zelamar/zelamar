import { env, exports } from 'cloudflare:workers';
import { describe, expect, it } from 'vitest';

import packageJson from '../package.json';
import worker from '../src/index';

const IncomingRequest = Request<unknown, IncomingRequestCfProperties>;

describe('GET /health', () => {
  it('reports the service as healthy (unit)', async () => {
    const response = await worker.fetch(
      new IncomingRequest('https://api.zelamar.test/health'),
      env,
    );

    expect(response.status).toBe(200);
    expect(await response.json()).toEqual({
      status: 'ok',
      service: 'zelamar',
      version: packageJson.version,
    });
  });

  it('is never cached and is served as JSON (integration)', async () => {
    const response = await exports.default.fetch('https://api.zelamar.test/health');

    expect(response.status).toBe(200);
    expect(response.headers.get('content-type')).toBe('application/json; charset=utf-8');
    expect(response.headers.get('cache-control')).toBe('no-store');
    expect(response.headers.get('x-content-type-options')).toBe('nosniff');
  });

  it('rejects mutating methods with 405 and an Allow header', async () => {
    const response = await exports.default.fetch('https://api.zelamar.test/health', {
      method: 'POST',
    });

    expect(response.status).toBe(405);
    expect(response.headers.get('allow')).toBe('GET, HEAD');
    expect(await response.json()).toEqual({ error: 'method_not_allowed' });
  });
});

describe('unknown routes', () => {
  it('return a JSON 404', async () => {
    const response = await exports.default.fetch('https://api.zelamar.test/nope');

    expect(response.status).toBe(404);
    expect(await response.json()).toEqual({ error: 'not_found' });
  });
});
