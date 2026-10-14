import { env, exports } from 'cloudflare:workers';
import { runDurableObjectAlarm, runInDurableObject } from 'cloudflare:test';
import { describe, expect, it } from 'vitest';

import { concat, utf8 } from '../src/bytes';
import { encodeDelivery, encodeEnvelope } from '../src/frames';
import { WINDOW } from '../src/mailbox';
import {
  BASE,
  b64,
  connect,
  deliver,
  envelopeId,
  envelopePayload,
  inbox,
  Inbox,
  random,
  registered,
} from './helpers';

describe('delivering envelopes', () => {
  it('reaches the owner live, again after reconnecting, and never after acknowledging', async () => {
    const { device: phone } = await registered();
    const live = await inbox(phone);

    const id = random(16);
    const payload = utf8('an MLS message');
    const response = await deliver(id, [phone.mailbox], payload);
    expect(response.status).toBe(202);
    expect(await response.json()).toEqual({ delivered: 1 });
    expect(await live.next()).toEqual(encodeEnvelope(id, phone.mailbox, payload));
    live.close();

    // Not acknowledged: handed out again.
    const again = await inbox(phone);
    const redelivered = await again.next();
    expect(envelopeId(redelivered)).toEqual(id);
    again.ack(id);
    expect(await again.settle()).toBe(0);
    again.close();

    const later = random(16);
    await deliver(later, [phone.mailbox], utf8('later'));
    const fresh = await inbox(phone);
    expect(envelopeId(await fresh.next())).toEqual(later);
    expect(await fresh.settle()).toBe(0);
    fresh.close();
  });

  it('fans one payload out to every recipient and skips unknown mailboxes', async () => {
    const alice = await registered();
    const bob = await registered();
    const id = random(16);
    const payload = utf8('to both');
    const response = await deliver(
      id,
      [alice.device.mailbox, random(32), bob.device.mailbox],
      payload,
    );
    expect(await response.json()).toEqual({ delivered: 2 });

    for (const person of [alice, bob]) {
      const live = await inbox(person.device);
      const envelope = await live.next();
      expect(envelopeId(envelope)).toEqual(id);
      expect(envelopePayload(envelope)).toEqual(payload);
      live.close();
    }
  });

  it('queues a retried envelope only once', async () => {
    const { device: phone } = await registered();
    const id = random(16);
    await deliver(id, [phone.mailbox], utf8('once'));
    const retry = await deliver(id, [phone.mailbox], utf8('once'));
    expect(await retry.json()).toEqual({ delivered: 1 });

    const live = await inbox(phone);
    expect(envelopeId(await live.next())).toEqual(id);
    expect(await live.settle()).toBe(0);
    live.close();
  });

  it(`keeps at most ${String(WINDOW)} unacknowledged envelopes in flight per connection`, async () => {
    const { device: phone } = await registered();
    const ids = Array.from({ length: WINDOW + 6 }, () => random(16));
    for (const id of ids) {
      await deliver(id, [phone.mailbox], id);
    }

    const live = await inbox(phone);
    const first: Uint8Array[] = [];
    for (let i = 0; i < WINDOW; i++) {
      first.push(envelopeId(await live.next()));
    }
    expect(first).toEqual(ids.slice(0, WINDOW));
    expect(await live.settle()).toBe(0);

    live.ack(...first.slice(0, 6));
    for (const id of ids.slice(WINDOW)) {
      expect(envelopeId(await live.next())).toEqual(id);
    }
    live.close();
  });

  it('drops envelopes nobody collected before they expire', async () => {
    const { device: phone } = await registered();
    await deliver(random(16), [phone.mailbox], utf8('too late'));

    const mailbox = env.MAILBOX.getByName(b64(phone.mailbox));
    await runInDurableObject(mailbox, (_, state) => {
      state.storage.sql.exec('UPDATE envelopes SET expires = 0');
    });
    await runDurableObjectAlarm(mailbox);

    const live = await inbox(phone);
    expect(await live.settle()).toBe(0);
    live.close();
  });
});

describe('reading a mailbox', () => {
  it('requires the mailbox secret', async () => {
    const { device: phone } = await registered();
    const wrong = await connect(phone.mailbox, random(32));
    expect(wrong).not.toBeInstanceOf(Inbox);
    expect((wrong as Response).status).toBe(401);
  });

  it('does not exist until a device registers it', async () => {
    const response = await connect(random(32), random(32));
    expect((response as Response).status).toBe(404);
  });

  it('only speaks WebSocket', async () => {
    const { device: phone } = await registered();
    const plain = await exports.default.fetch(`${BASE}/v0/mailboxes/${b64(phone.mailbox)}`);
    expect(plain.status).toBe(426);
  });

  it('closes a connection that sends anything but acknowledgements', async () => {
    const { device: phone } = await registered();
    const live = await inbox(phone);
    const closed = new Promise<number>((resolve) => {
      live.socket.addEventListener('close', (event) => {
        resolve(event.code);
      });
    });
    live.socket.send('hello');
    expect(await closed).toBe(1008);
  });
});

describe('malformed deliveries', () => {
  const post = (body: Uint8Array): Promise<Response> =>
    exports.default.fetch(`${BASE}/v0/deliver`, { method: 'POST', body });
  const valid = (): Uint8Array =>
    encodeDelivery({ id: random(16), recipients: [random(32)], payload: utf8('x') });

  it('are rejected without touching any mailbox', async () => {
    const wrongMagic = valid();
    wrongMagic[0] = 0;
    const truncated = valid().subarray(0, 30);
    const trailing = concat(valid(), Uint8Array.of(0));
    const noRecipients = encodeDelivery({ id: random(16), recipients: [], payload: utf8('x') });
    const twice = random(32);
    const duplicates = encodeDelivery({
      id: random(16),
      recipients: [twice, twice],
      payload: utf8('x'),
    });

    for (const frame of [wrongMagic, truncated, trailing, noRecipients, duplicates]) {
      const response = await post(frame);
      expect(response.status).toBe(400);
      expect(await response.json()).toEqual({ error: 'malformed' });
    }
  });

  it('refuse oversized payloads', async () => {
    const frame = valid();
    // Declare a payload one byte over the limit.
    new DataView(frame.buffer).setUint32(2 + 1 + 16 + 1 + 32, (1 << 20) + 1);
    expect((await post(frame)).status).toBe(413);
  });

  it('only accept POST', async () => {
    const response = await exports.default.fetch(`${BASE}/v0/deliver`);
    expect(response.status).toBe(405);
    expect(response.headers.get('allow')).toBe('POST');
  });
});
