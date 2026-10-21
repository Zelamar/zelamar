import { exports } from 'cloudflare:workers';
import { describe, expect, it } from 'vitest';

import { MAX_DIRECTORY_BATCH, MAX_RECOVERY_KIT } from '../src/account';
import { utf8 } from '../src/bytes';
import { requestPayload, sha256 } from '../src/crypto';
import {
  address,
  b64,
  BASE,
  device,
  devicePath,
  get,
  type Keys,
  post,
  random,
  registered,
  registrationBody,
  sign,
  signedFetch,
  username,
} from './helpers';

const recoveryPath = (name: string): string => `/v0/accounts/${name}/recovery`;

/** Leaves a kit, signed by the account key as `protocol/recovery.md` says. */
async function saveKit(
  account: Keys,
  name: string,
  body: unknown,
  timestamp = Date.now(),
): Promise<Response> {
  const path = recoveryPath(name);
  const bytes = utf8(JSON.stringify(body));
  const payload = await requestPayload('PUT', path, timestamp, bytes, 'zelamar/account-request/v0');
  return exports.default.fetch(BASE + path, {
    method: 'PUT',
    body: bytes,
    headers: {
      'zelamar-timestamp': String(timestamp),
      'zelamar-account-signature': b64(await sign(account, payload)),
    },
  });
}

function fetchKit(name: string, token: unknown, from = address()): Promise<Response> {
  return exports.default.fetch(BASE + recoveryPath(name), {
    method: 'POST',
    body: JSON.stringify({ token }),
    headers: { 'cf-connecting-ip': from },
  });
}

async function kitFor(token: Uint8Array, kit: Uint8Array): Promise<Record<string, string>> {
  return { token_hash: b64(await sha256(token)), kit: b64(kit) };
}

describe('recovery kits', () => {
  it('keep the newest kit and hand it only to its token', async () => {
    const { name, account } = await registered();
    const token = random(32);
    const kit = random(500);
    expect(await (await fetchKit(name, b64(token))).json()).toEqual({ error: 'wrong_key' });

    expect((await saveKit(account, name, await kitFor(token, kit))).status).toBe(200);
    const fetched = await fetchKit(name, b64(token));
    expect(fetched.status).toBe(200);
    expect(await fetched.json()).toEqual({ kit: b64(kit) });
    const wrong = await fetchKit(name, b64(random(32)));
    expect(wrong.status).toBe(403);
    expect(await wrong.json()).toEqual({ error: 'wrong_key' });

    // A new key replaces the old one at once.
    const next = random(32);
    const nextKit = random(100);
    expect((await saveKit(account, name, await kitFor(next, nextKit))).status).toBe(200);
    expect((await fetchKit(name, b64(token))).status).toBe(403);
    expect(await (await fetchKit(name, b64(next))).json()).toEqual({ kit: b64(nextKit) });

    // A name nobody holds looks the same as a wrong key.
    expect(await (await fetchKit(username(), b64(next))).json()).toEqual({
      error: 'wrong_key',
    });
  });

  it('are left only by the account key, fresh, and within the size limit', async () => {
    const { name, account, device: phone } = await registered();
    const stranger = await registered();
    const body = await kitFor(random(32), random(64));
    expect((await saveKit(stranger.account, name, body)).status).toBe(401);
    expect((await saveKit(account, name, body, Date.now() - 600_000)).status).toBe(400);
    // A device's own signature is not the account's.
    expect((await signedFetch(phone, 'PUT', recoveryPath(name), body)).status).toBe(401);
    const large = { ...body, kit: b64(new Uint8Array(MAX_RECOVERY_KIT + 1).fill(7)) };
    expect((await saveKit(account, name, large)).status).toBe(413);
    expect((await saveKit(account, name, { ...body, token_hash: 'short' })).status).toBe(400);
    expect((await saveKit(account, name, { ...body, kit: '' })).status).toBe(400);
    expect((await saveKit(account, username(), body)).status).toBe(404);
    expect((await fetchKit(name, 'short')).status).toBe(400);
    expect((await get(recoveryPath(name))).status).toBe(405);
  });

  it('are fetched at most ten times a minute from one address', async () => {
    const { name } = await registered();
    const from = address();
    const statuses: number[] = [];
    for (let attempt = 0; attempt < 11; attempt += 1) {
      statuses.push((await fetchKit(name, b64(random(32)), from)).status);
    }
    expect(statuses.slice(0, 10)).toEqual(Array<number>(10).fill(403));
    expect(statuses[10]).toBe(429);
  });
});

interface Listed {
  username: string;
  account: string;
  devices: { certificate: string }[];
}

describe('device lists for many names', () => {
  it('list the registered devices of every name held, in one request', async () => {
    const alice = await registered();
    const bob = await registered();
    const tablet = await device(bob.account);
    await signedFetch(
      tablet,
      'PUT',
      devicePath(bob.name, tablet.id),
      await registrationBody(tablet),
    );

    const response = await post('/v0/directory/devices', {
      usernames: [alice.name, bob.name, username(), alice.name],
    });
    expect(response.status).toBe(200);
    const { accounts } = await response.json<{ accounts: Listed[] }>();
    expect(accounts).toEqual([
      {
        username: alice.name,
        account: b64(alice.account.publicKey),
        devices: [{ certificate: b64(alice.device.certificate) }],
      },
      {
        username: bob.name,
        account: b64(bob.account.publicKey),
        devices: [
          { certificate: b64(bob.device.certificate) },
          { certificate: b64(tablet.certificate) },
        ],
      },
    ]);
  });

  it('refuse no names, too many, and anything but normalized names', async () => {
    const lists = (usernames: unknown): Promise<Response> =>
      post('/v0/directory/devices', { usernames });
    expect((await lists([])).status).toBe(400);
    const many = Array.from({ length: MAX_DIRECTORY_BATCH + 1 }, () => username());
    expect((await lists(many)).status).toBe(400);
    expect((await lists(many.slice(1))).status).toBe(200);
    expect((await lists(['Alice'])).status).toBe(400);
    expect((await lists([42])).status).toBe(400);
    expect((await lists('alice')).status).toBe(400);
    expect((await get('/v0/directory/devices')).status).toBe(405);
  });
});
