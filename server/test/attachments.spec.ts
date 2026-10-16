import { runDurableObjectAlarm, runInDurableObject } from 'cloudflare:test';
import { env, exports } from 'cloudflare:workers';
import { describe, expect, it } from 'vitest';

import { MAX_CHUNK } from '../src/blob';
import { UPLOAD_QUOTA } from '../src/mailbox';
import { address, b64, BASE, type Device, random, registered } from './helpers';

function put(
  device: Pick<Device, 'mailbox' | 'secret'>,
  blob: string,
  index: number | string,
  body: Uint8Array,
  count: number | string = 2,
): Promise<Response> {
  return exports.default.fetch(`${BASE}/v0/blobs/${blob}/${String(index)}`, {
    method: 'PUT',
    body,
    headers: {
      authorization: `Bearer ${b64(device.secret)}`,
      'zelamar-mailbox': b64(device.mailbox),
      'zelamar-chunks': String(count),
      'cf-connecting-ip': address(),
    },
  });
}

/** `length` bytes of noise; `random` stops at 64 KiB. */
function noise(length: number): Uint8Array {
  const out = new Uint8Array(length);
  for (let at = 0; at < length; at += 65_536) {
    out.set(random(Math.min(65_536, length - at)), at);
  }
  return out;
}

function get(blob: string, index: number | string): Promise<Response> {
  return exports.default.fetch(`${BASE}/v0/blobs/${blob}/${String(index)}`, {
    headers: { 'cf-connecting-ip': address() },
  });
}

describe('file chunks', () => {
  it('are stored once by a registered device and handed to anyone with the id', async () => {
    const { device: phone } = await registered();
    const blob = b64(random(16));
    const first = random(1000);

    expect((await get(blob, 0)).status).toBe(404);
    expect((await put(phone, blob, 0, first)).status).toBe(201);
    expect((await put(phone, blob, 0, first)).status).toBe(200);
    const rival = await put(phone, blob, 0, random(1000));
    expect(rival.status).toBe(409);
    expect(await rival.json()).toEqual({ error: 'blob_taken' });
    // Every chunk of a blob declares the same count.
    expect((await put(phone, blob, 1, random(100), 3)).status).toBe(409);

    const got = await get(blob, 0);
    expect(got.status).toBe(200);
    expect(got.headers.get('content-type')).toBe('application/octet-stream');
    expect(new Uint8Array(await got.arrayBuffer())).toEqual(first);
    expect((await get(blob, 1)).status).toBe(404);

    const second = noise(MAX_CHUNK);
    expect((await put(phone, blob, 1, second)).status).toBe(201);
    expect(new Uint8Array(await (await get(blob, 1)).arrayBuffer())).toEqual(second);
  }, 30_000);

  it('refuse strangers, bad sizes and bad places', async () => {
    const { device: phone } = await registered();
    const blob = b64(random(16));
    const chunk = random(64);

    const wrongSecret = { ...phone, secret: random(32) };
    expect((await put(wrongSecret, blob, 0, chunk)).status).toBe(401);
    const unknownMailbox = { mailbox: random(32), secret: random(32) };
    expect((await put(unknownMailbox, blob, 0, chunk)).status).toBe(401);

    expect((await put(phone, blob, 0, chunk, 0)).status).toBe(400);
    expect((await put(phone, blob, 0, chunk, 26)).status).toBe(400);
    expect((await put(phone, blob, 2, chunk, 2)).status).toBe(400);
    expect((await put(phone, blob, 0, chunk, 'two')).status).toBe(400);
    expect((await put(phone, blob, 0, random(15))).status).toBe(400);
    expect((await put(phone, blob, 0, noise(MAX_CHUNK + 1))).status).toBe(413);
    expect((await put(phone, 'short', 0, chunk)).status).toBe(404);
    expect((await put(phone, blob, 'x', chunk)).status).toBe(404);
    expect((await put(phone, blob, 25, chunk)).status).toBe(404);
    expect((await get(blob, 0)).status).toBe(404);
    expect(
      (await exports.default.fetch(`${BASE}/v0/blobs/${blob}/0`, { method: 'POST' })).status,
    ).toBe(405);
  });

  it('count against a daily quota per mailbox', async () => {
    const { device: phone } = await registered();
    const mailbox = env.MAILBOX.getByName(b64(phone.mailbox));
    expect((await put(phone, b64(random(16)), 0, random(1000))).status).toBe(201);
    await runInDurableObject(mailbox, (_, state) => {
      state.storage.sql.exec('UPDATE uploads SET bytes = ?', UPLOAD_QUOTA - 500);
    });
    const over = await put(phone, b64(random(16)), 0, random(1000));
    expect(over.status).toBe(429);
    expect(await over.json()).toEqual({ error: 'over_quota' });
    expect((await put(phone, b64(random(16)), 0, random(400))).status).toBe(201);
  });

  it('expire', async () => {
    const { device: phone } = await registered();
    const blob = b64(random(16));
    await put(phone, blob, 0, random(100));
    const stub = env.BLOB.getByName(blob);

    await runInDurableObject(stub, (_, state) => {
      state.storage.sql.exec('UPDATE blob SET expires = 0');
    });
    expect((await get(blob, 0)).status).toBe(404);
    await runDurableObjectAlarm(stub);
    expect((await get(blob, 0)).status).toBe(404);
  });
});
