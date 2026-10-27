import { runDurableObjectAlarm, runInDurableObject } from 'cloudflare:test';
import { env, exports } from 'cloudflare:workers';
import { describe, expect, it } from 'vitest';

import { fromBase64Url, utf8 } from '../src/bytes';
import { assertion, serviceAccount, wakeUp } from '../src/push';
import { address, b64, BASE, deliver, type Device, inbox, random, registered } from './helpers';

function setPush(device: Pick<Device, 'mailbox' | 'secret'>, body?: unknown): Promise<Response> {
  return exports.default.fetch(`${BASE}/v0/mailboxes/${b64(device.mailbox)}/push`, {
    method: body === undefined ? 'DELETE' : 'PUT',
    body: body === undefined ? undefined : JSON.stringify(body),
    headers: { authorization: `Bearer ${b64(device.secret)}`, 'cf-connecting-ip': address() },
  });
}

/** How many wake-ups the push object skipped (no Firebase in tests). */
function skipped(wake: 'message' | 'call' = 'message'): Promise<number> {
  return env.PUSH.getByName('fcm').skipped(wake);
}

/**
 * Lets the mailbox's alarm send an owed wake-up. An alarm due now may
 * already be running on its own; it is done once it has rescheduled
 * itself for the envelopes' expiry.
 */
async function tick(device: Device): Promise<void> {
  const stub = env.MAILBOX.getByName(b64(device.mailbox));
  await runDurableObjectAlarm(stub);
  for (let attempt = 0; attempt < 200; attempt++) {
    const alarm = await runInDurableObject(stub, (_, state) => state.storage.getAlarm());
    if (alarm === null || alarm > Date.now() + 60_000) {
      return;
    }
    await new Promise((resolve) => setTimeout(resolve, 10));
  }
  throw new Error('the mailbox alarm never finished');
}

describe('push wake-ups', () => {
  it('go to a device that is away, at most once per interval', async () => {
    const { device: phone } = await registered();
    expect((await setPush(phone, { platform: 'fcm', token: 'device-token' })).status).toBe(200);
    const before = await skipped();

    await deliver(random(16), [phone.mailbox], utf8('one'));
    await tick(phone);
    expect(await skipped()).toBe(before + 1);

    // A burst is one wake-up.
    await deliver(random(16), [phone.mailbox], utf8('two'));
    await tick(phone);
    expect(await skipped()).toBe(before + 1);

    // After the interval, the next message wakes it again.
    await runInDurableObject(env.MAILBOX.getByName(b64(phone.mailbox)), (_, state) => {
      state.storage.sql.exec('UPDATE push SET last = 0');
    });
    await deliver(random(16), [phone.mailbox], utf8('three'));
    await tick(phone);
    expect(await skipped()).toBe(before + 2);
  });

  it('are not sent while the device is connected, or without a token', async () => {
    const { device: phone } = await registered();
    await setPush(phone, { platform: 'fcm', token: 'device-token' });
    const before = await skipped();
    const live = await inbox(phone);
    await deliver(random(16), [phone.mailbox], utf8('read at once'));
    await live.next();
    await tick(phone);
    expect(await skipped()).toBe(before);
    live.close();

    const { device: quiet } = await registered();
    await deliver(random(16), [quiet.mailbox], utf8('no token'));
    await tick(quiet);
    expect(await skipped()).toBe(before);

    expect((await setPush(phone)).status).toBe(200);
    await deliver(random(16), [phone.mailbox], utf8('forgotten'));
    await tick(phone);
    expect(await skipped()).toBe(before);
  });

  it('wake a phone at once for a call, apart from messages', async () => {
    const { device: phone } = await registered();
    await setPush(phone, { platform: 'fcm', token: 'device-token' });
    const calls = await skipped('call');
    const messages = await skipped();
    const ring = { 'zelamar-wake': 'call' };

    await deliver(random(16), [phone.mailbox], utf8('offer'), ring);
    await tick(phone);
    expect(await skipped('call')).toBe(calls + 1);
    // A call wake-up is not a message one.
    expect(await skipped()).toBe(messages);

    // A second offer right away is the same ring.
    await deliver(random(16), [phone.mailbox], utf8('again'), ring);
    await tick(phone);
    expect(await skipped('call')).toBe(calls + 1);

    // Seconds later, a new call wakes it again, even right after a message.
    await runInDurableObject(env.MAILBOX.getByName(b64(phone.mailbox)), (_, state) => {
      state.storage.sql.exec('UPDATE call_wake SET last = 0');
      state.storage.sql.exec('UPDATE push SET last = ?', Date.now());
    });
    await deliver(random(16), [phone.mailbox], utf8('ring'), ring);
    await tick(phone);
    expect(await skipped('call')).toBe(calls + 2);

    // Any other value is an ordinary message.
    const { device: other } = await registered();
    await setPush(other, { platform: 'fcm', token: 'other-token' });
    await deliver(random(16), [other.mailbox], utf8('hi'), { 'zelamar-wake': 'yes' });
    await tick(other);
    expect(await skipped('call')).toBe(calls + 2);
    expect(await skipped()).toBe(messages + 1);
  });

  it('do not ring a phone that is connected', async () => {
    const { device: phone } = await registered();
    await setPush(phone, { platform: 'fcm', token: 'device-token' });
    const calls = await skipped('call');
    const live = await inbox(phone);
    await deliver(random(16), [phone.mailbox], utf8('offer'), { 'zelamar-wake': 'call' });
    await live.next();
    await tick(phone);
    expect(await skipped('call')).toBe(calls);
    live.close();
  });

  it('take a token only from the mailbox owner, and only a plain one', async () => {
    const { device: phone } = await registered();
    expect(
      (await setPush({ ...phone, secret: random(32) }, { platform: 'fcm', token: 't' })).status,
    ).toBe(401);
    expect(
      (await setPush({ mailbox: random(32), secret: random(32) }, { platform: 'fcm', token: 't' }))
        .status,
    ).toBe(401);
    expect((await setPush(phone, { platform: 'apns', token: 't' })).status).toBe(400);
    expect((await setPush(phone, { platform: 'fcm', token: '' })).status).toBe(400);
    expect((await setPush(phone, { platform: 'fcm', token: 'x'.repeat(4097) })).status).toBe(400);
    const get = await exports.default.fetch(`${BASE}/v0/mailboxes/${b64(phone.mailbox)}/push`);
    expect(get.status).toBe(405);
  });
});

describe('the Firebase sender', () => {
  it('signs a token request Google can check, for messaging only', async () => {
    const keys = (await crypto.subtle.generateKey(
      {
        name: 'RSASSA-PKCS1-v1_5',
        modulusLength: 2048,
        publicExponent: new Uint8Array([1, 0, 1]),
        hash: 'SHA-256',
      },
      true,
      ['sign', 'verify'],
    )) as CryptoKeyPair;
    const pkcs8 = new Uint8Array(
      (await crypto.subtle.exportKey('pkcs8', keys.privateKey)) as ArrayBuffer,
    );
    const pem = `-----BEGIN PRIVATE KEY-----\n${btoa(String.fromCharCode(...pkcs8))}\n-----END PRIVATE KEY-----\n`;
    const account = serviceAccount(
      JSON.stringify({
        type: 'service_account',
        project_id: 'zelamar-test',
        client_email: 'push@zelamar-test.iam.gserviceaccount.com',
        private_key: pem,
      }),
    );
    expect(account).not.toBeNull();

    if (account === null) {
      throw new Error('the service account did not parse');
    }
    const jwt = await assertion(account, 1_760_000_000);
    const [header, claims, signature] = jwt.split('.');
    const decode = (part: string | undefined): unknown =>
      JSON.parse(new TextDecoder().decode(fromBase64Url(part ?? '') ?? new Uint8Array()));
    expect(decode(header)).toEqual({ alg: 'RS256', typ: 'JWT' });
    expect(decode(claims)).toEqual({
      iss: 'push@zelamar-test.iam.gserviceaccount.com',
      scope: 'https://www.googleapis.com/auth/firebase.messaging',
      aud: 'https://oauth2.googleapis.com/token',
      iat: 1_760_000_000,
      exp: 1_760_003_600,
    });
    const valid = await crypto.subtle.verify(
      'RSASSA-PKCS1-v1_5',
      keys.publicKey,
      fromBase64Url(signature ?? '') ?? new Uint8Array(),
      utf8(`${header ?? ''}.${claims ?? ''}`),
    );
    expect(valid).toBe(true);
  });

  it('sends the same words every time, and nothing else', () => {
    const message = JSON.stringify(wakeUp('token-a'));
    expect(message).toBe(JSON.stringify(wakeUp('token-a')));
    expect(message).toContain('"title":"Zelamar"');
    expect(message).toContain('"body":"New message"');
    expect(message).not.toContain('data');
  });

  it('wakes for a call without a word, high priority, briefly', () => {
    expect(wakeUp('token-a', 'call')).toEqual({
      message: {
        token: 'token-a',
        data: { wake: 'call' },
        android: { priority: 'HIGH', ttl: '45s' },
      },
    });
  });

  it('treats a missing or broken service account as unconfigured', () => {
    expect(serviceAccount(undefined)).toBeNull();
    expect(serviceAccount('')).toBeNull();
    expect(serviceAccount('{not json')).toBeNull();
    expect(serviceAccount('{"project_id": 1}')).toBeNull();
  });
});
