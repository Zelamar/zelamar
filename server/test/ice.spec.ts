import { runInDurableObject } from 'cloudflare:test';
import { env, exports } from 'cloudflare:workers';
import { describe, expect, it } from 'vitest';

import { iceServers, STUN, TURN_TTL, turnKey } from '../src/ice';
import { ICE_QUOTA } from '../src/mailbox';
import { address, b64, BASE, type Device, random, registered } from './helpers';

function askIce(device: Pick<Device, 'mailbox' | 'secret'>, secret = device.secret) {
  return exports.default.fetch(`${BASE}/v0/mailboxes/${b64(device.mailbox)}/ice`, {
    method: 'POST',
    headers: { authorization: `Bearer ${b64(secret)}`, 'cf-connecting-ip': address() },
  });
}

describe('call servers', () => {
  it('go only to the device that owns the mailbox', async () => {
    const { device } = await registered();
    const response = await askIce(device);
    expect(response.status).toBe(200);
    // No TURN key in tests: STUN only.
    expect(await response.json()).toEqual({ ice_servers: [STUN] });

    expect((await askIce(device, random(32))).status).toBe(401);
    const stranger = { mailbox: random(32), secret: random(32) };
    expect((await askIce(stranger)).status).toBe(401);
    const get = await exports.default.fetch(`${BASE}/v0/mailboxes/${b64(device.mailbox)}/ice`, {
      headers: { authorization: `Bearer ${b64(device.secret)}` },
    });
    expect(get.status).toBe(405);
  });

  it('are rationed per mailbox and day', async () => {
    const { device } = await registered();
    expect((await askIce(device)).status).toBe(200);
    await runInDurableObject(env.MAILBOX.getByName(b64(device.mailbox)), (_, state) => {
      state.storage.sql.exec('UPDATE ice SET count = ?', ICE_QUOTA);
    });
    const over = await askIce(device);
    expect(over.status).toBe(429);
    expect(await over.json()).toEqual({ error: 'over_quota' });

    // A new day, a new allowance.
    await runInDurableObject(env.MAILBOX.getByName(b64(device.mailbox)), (_, state) => {
      state.storage.sql.exec('UPDATE ice SET day = day - 1');
    });
    expect((await askIce(device)).status).toBe(200);
  });

  it('include a short-lived relay once the service holds a TURN key', async () => {
    const asked: { url: string; init: RequestInit | undefined }[] = [];
    const cloudflare = ((input: RequestInfo | URL, init?: RequestInit) => {
      const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
      asked.push({ url, init });
      return Promise.resolve(
        Response.json(
          {
            iceServers: [
              { urls: ['stun:stun.cloudflare.com:3478', 'stun:stun.cloudflare.com:53'] },
              {
                urls: [
                  'turn:turn.cloudflare.com:3478?transport=udp',
                  'turn:turn.cloudflare.com:53?transport=udp',
                  'turns:turn.cloudflare.com:443?transport=tcp',
                ],
                username: 'short-lived',
                credential: 'secret',
              },
            ],
          },
          { status: 201 },
        ),
      );
    }) as typeof fetch;

    const servers = await iceServers({ id: 'key id', token: 'api-token' }, cloudflare);
    expect(servers).toEqual([
      { urls: ['stun:stun.cloudflare.com:3478'] },
      {
        urls: [
          'turn:turn.cloudflare.com:3478?transport=udp',
          'turns:turn.cloudflare.com:443?transport=tcp',
        ],
        username: 'short-lived',
        credential: 'secret',
      },
    ]);
    expect(asked).toHaveLength(1);
    expect(asked[0]?.url).toBe(
      'https://rtc.live.cloudflare.com/v1/turn/keys/key%20id/credentials/generate-ice-servers',
    );
    expect(asked[0]?.init?.method).toBe('POST');
    expect(new Headers(asked[0]?.init?.headers).get('authorization')).toBe('Bearer api-token');
    const body = asked[0]?.init?.body;
    expect(JSON.parse(typeof body === 'string' ? body : '')).toEqual({ ttl: TURN_TTL });
  });

  it('fall back to STUN when the relay cannot be reached', async () => {
    const down = (() => Promise.resolve(new Response('nope', { status: 503 }))) as typeof fetch;
    expect(await iceServers({ id: 'k', token: 't' }, down)).toEqual([STUN]);
    const broken = (() => Promise.resolve(Response.json({ iceServers: [] }))) as typeof fetch;
    expect(await iceServers({ id: 'k', token: 't' }, broken)).toEqual([STUN]);
    const offline = (() => Promise.reject(new Error('offline'))) as typeof fetch;
    expect(await iceServers({ id: 'k', token: 't' }, offline)).toEqual([STUN]);
  });

  it('need both halves of the TURN key', () => {
    expect(turnKey('', '')).toBeNull();
    expect(turnKey('id', '')).toBeNull();
    expect(turnKey(undefined, 'token')).toBeNull();
    expect(turnKey(' id ', ' token ')).toEqual({ id: 'id', token: 'token' });
  });
});
