import { runDurableObjectAlarm, runInDurableObject } from 'cloudflare:test';
import { env, exports } from 'cloudflare:workers';
import { describe, expect, it } from 'vitest';

import { concat, utf8 } from '../src/bytes';
import { MAX_LINK_ANSWER, MAX_LINK_REQUEST } from '../src/link';
import {
  address,
  b64,
  BASE,
  device,
  devicePath,
  get,
  random,
  registered,
  registrationBody,
  signedFetch,
} from './helpers';

const keyPackage = (label: string): Uint8Array => utf8(`key-package-${label}`);

interface Listed {
  username: string;
  account: string;
  devices: { certificate: string; key_package?: string }[];
}

/** A sealed link message as far as the service can tell: header, then noise. */
function sealed(kind: 1 | 2, size = 120): Uint8Array {
  return concat(Uint8Array.of(0x5a, 0x4c, 0x00, kind), random(size - 4));
}

function link(method: string, path: string, body?: Uint8Array): Promise<Response> {
  return exports.default.fetch(`${BASE}/v0/links/${path}`, {
    method,
    body,
    headers: { 'cf-connecting-ip': address() },
  });
}

describe('linked devices in the directory', () => {
  it('lists every device without handing out key packages', async () => {
    const { name, account, device: first } = await registered([keyPackage('a1')]);
    const second = await device(account);
    await signedFetch(
      second,
      'PUT',
      devicePath(name, second.id),
      await registrationBody(second, [keyPackage('b1')]),
    );

    for (let i = 0; i < 2; i++) {
      const listed = await (await get(`/v0/accounts/${name}/devices`)).json<Listed>();
      expect(listed).toEqual({
        username: name,
        account: b64(account.publicKey),
        devices: [
          { certificate: b64(first.certificate) },
          { certificate: b64(second.certificate) },
        ],
      });
    }

    // One device's key package, then its last resort; the other's is untouched.
    const one = await (await get(devicePath(name, second.id))).json<Listed>();
    expect(one.devices).toEqual([
      { certificate: b64(second.certificate), key_package: b64(keyPackage('b1')) },
    ]);
    const again = await (await get(devicePath(name, second.id))).json<Listed>();
    expect(again.devices[0]?.key_package).toBe(b64(utf8(`last-resort-${b64(second.id)}`)));
    const all = await (await get(`/v0/accounts/${name}`)).json<Listed>();
    expect(all.devices[0]?.key_package).toBe(b64(keyPackage('a1')));
  });

  it('answers unknown devices and accounts without revealing anything', async () => {
    const { name } = await registered();
    expect((await get(devicePath(name, random(16)))).status).toBe(404);
    expect(await (await get(`/v0/accounts/${name}/devices/short`)).json()).toEqual({
      error: 'unknown_device',
    });
    expect((await get('/v0/accounts/nobodyhere/devices')).status).toBe(404);
    expect((await get('/v0/accounts/Not-A-Name!/devices')).status).toBe(400);
  });
});

describe('link slots', () => {
  it('keep the first request and the first answer, then forget everything', async () => {
    const slot = b64(random(16));
    expect((await link('GET', slot)).status).toBe(404);
    // No answer before a request.
    expect((await link('PUT', `${slot}/answer`, sealed(2))).status).toBe(404);

    const request = sealed(1);
    expect((await link('PUT', `${slot}/request`, request)).status).toBe(201);
    expect((await link('PUT', `${slot}/request`, request)).status).toBe(200);
    const rival = await link('PUT', `${slot}/request`, sealed(1));
    expect(rival.status).toBe(409);
    expect(await rival.json()).toEqual({ error: 'link_taken' });
    expect(await (await link('GET', slot)).json()).toEqual({ request: b64(request), answer: null });

    const answer = sealed(2, 4000);
    expect((await link('PUT', `${slot}/answer`, answer)).status).toBe(201);
    expect((await link('PUT', `${slot}/answer`, answer)).status).toBe(200);
    expect((await link('PUT', `${slot}/answer`, sealed(2))).status).toBe(409);
    expect(await (await link('GET', slot)).json()).toEqual({
      request: b64(request),
      answer: b64(answer),
    });

    expect((await link('DELETE', slot)).status).toBe(204);
    expect((await link('GET', slot)).status).toBe(404);
  });

  it('expire ten minutes after the request', async () => {
    const slot = b64(random(16));
    await link('PUT', `${slot}/request`, sealed(1));
    const stub = env.LINK.getByName(slot);

    // Expired but not yet swept: already gone, and the slot can start over.
    await runInDurableObject(stub, (_, state) => {
      state.storage.sql.exec('UPDATE slot SET expires = 0');
    });
    expect((await link('GET', slot)).status).toBe(404);
    const fresh = sealed(1);
    expect((await link('PUT', `${slot}/request`, fresh)).status).toBe(201);
    expect(await (await link('GET', slot)).json()).toEqual({ request: b64(fresh), answer: null });

    await runDurableObjectAlarm(stub);
    expect((await link('GET', slot)).status).toBe(404);
  });

  it('take only sealed link messages of the right kind and size', async () => {
    const slot = b64(random(16));
    expect((await link('PUT', `${slot}/request`, sealed(2))).status).toBe(400);
    expect((await link('PUT', `${slot}/request`, utf8('{"hello":1}'))).status).toBe(400);
    expect((await link('PUT', `${slot}/request`, sealed(1, MAX_LINK_REQUEST + 1))).status).toBe(
      400,
    );
    expect((await link('PUT', `${slot}/request`, sealed(1, MAX_LINK_REQUEST))).status).toBe(201);
    expect((await link('PUT', `${slot}/answer`, sealed(2, MAX_LINK_ANSWER + 1))).status).toBe(413);
    expect((await link('PUT', `${slot}/answer`, sealed(1))).status).toBe(400);

    expect((await link('GET', 'short')).status).toBe(404);
    expect((await link('PUT', 'short/request', sealed(1))).status).toBe(404);
    expect((await link('POST', slot)).status).toBe(405);
    expect((await link('GET', `${slot}/request`)).status).toBe(405);
    expect((await link('PUT', `${slot}/other`, sealed(1))).status).toBe(404);
  });
});
