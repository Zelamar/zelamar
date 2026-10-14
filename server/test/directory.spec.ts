import { describe, expect, it } from 'vitest';

import { utf8 } from '../src/bytes';
import { MAX_DEVICES, MAX_KEY_PACKAGES } from '../src/account';
import {
  b64,
  claimBody,
  device,
  devicePath,
  get,
  keys,
  post,
  random,
  registered,
  registrationBody,
  signedFetch,
  username,
  work,
} from './helpers';

const keyPackage = (label: string): Uint8Array => utf8(`key-package-${label}`);

interface Lookup {
  username: string;
  account: string;
  devices: { certificate: string; key_package: string }[];
}

const lookup = async (name: string): Promise<Lookup> =>
  (await get(`/v0/accounts/${name}`)).json<Lookup>();

describe('claiming a username', () => {
  it('succeeds once, is idempotent for its owner, and refuses everyone else', async () => {
    const name = username();
    const alice = await keys();
    expect((await post('/v0/accounts', await claimBody(alice, name))).status).toBe(201);
    expect((await post('/v0/accounts', await claimBody(alice, name))).status).toBe(200);

    const mallory = await keys();
    const taken = await post('/v0/accounts', await claimBody(mallory, name));
    expect(taken.status).toBe(409);
    expect(await taken.json()).toEqual({ error: 'username_taken' });
  });

  it('is case-insensitive and stores the lowercase name', async () => {
    const name = username();
    const alice = await keys();
    const body = await claimBody(alice, name);
    const response = await post('/v0/accounts', { ...body, username: name.toUpperCase() });
    // The signature covers the lowercase name, which is what the service checks.
    expect(response.status).toBe(201);
    expect(await response.json()).toEqual({ username: name });
  });

  it('spends an invite exactly once', async () => {
    const first = await claimBody(await keys(), username());
    expect((await post('/v0/accounts', first)).status).toBe(201);
    const second = await claimBody(await keys(), username(), { invite: first.invite });
    const reused = await post('/v0/accounts', second);
    expect(reused.status).toBe(403);
    expect(await reused.json()).toEqual({ error: 'invalid_invite' });
  });

  it('refuses forged invites', async () => {
    const forged = await claimBody(await keys(), username(), {
      invite: 'aaaaa-aaaaa-aaaaa-aaaaa',
    });
    expect((await post('/v0/accounts', forged)).status).toBe(403);
  });

  it.each([
    ['ab', 'invalid_username', 400],
    ['1abc', 'invalid_username', 400],
    ['abc_', 'invalid_username', 400],
    ['a__b', 'invalid_username', 400],
    ['ab-c', 'invalid_username', 400],
    ['a'.repeat(33), 'invalid_username', 400],
    ['admin', 'reserved', 403],
    ['the_zelamar_team', 'reserved', 403],
  ])('refuses %s with %s', async (name, error, status) => {
    const response = await post(
      '/v0/accounts',
      await claimBody(await keys(), 'placeholder', { username: name }),
    );
    expect(response.status).toBe(status);
    expect(await response.json()).toEqual({ error });
  });

  it('requires the proof of work, a fresh timestamp and the account signature', async () => {
    const alice = await keys();
    const name = username();

    const lazy = await claimBody(alice, name, { work: b64(await work(alice, name, false)) });
    expect(await (await post('/v0/accounts', lazy)).json()).toEqual({ error: 'insufficient_work' });

    const stale = await claimBody(alice, name, {}, Date.now() - 10 * 60 * 1000);
    expect(await (await post('/v0/accounts', stale)).json()).toEqual({ error: 'stale_request' });

    // A valid proof of work, but a signature that is not the account's.
    const forged = await claimBody(alice, name, { signature: b64(random(64)) });
    const response = await post('/v0/accounts', forged);
    expect(response.status).toBe(401);
    expect(await response.json()).toEqual({ error: 'bad_signature' });
  });

  it('rejects malformed bodies', async () => {
    expect((await post('/v0/accounts', ['not', 'an', 'object'])).status).toBe(400);
    const missing = await claimBody(await keys(), username(), { signature: undefined });
    expect((await post('/v0/accounts', missing)).status).toBe(400);
  });
});

describe('devices and key packages', () => {
  it('registers a device and hands out each key package once, then the last resort', async () => {
    const { name, account, device: phone } = await registered([keyPackage('1'), keyPackage('2')]);

    const first = await lookup(name);
    expect(first.username).toBe(name);
    expect(first.account).toBe(b64(account.publicKey));
    expect(first.devices).toEqual([
      { certificate: b64(phone.certificate), key_package: b64(keyPackage('1')) },
    ]);

    const second = await lookup(name);
    expect(second.devices[0]?.key_package).toBe(b64(keyPackage('2')));

    const lastResort = b64(utf8(`last-resort-${b64(phone.id)}`));
    for (let i = 0; i < 2; i++) {
      const later = await lookup(name);
      expect(later.devices[0]?.key_package).toBe(lastResort);
    }
  });

  it('tells whose a name is without handing out key packages', async () => {
    const { name, account } = await registered([keyPackage('only')]);
    for (let i = 0; i < 3; i++) {
      const key = await get(`/v0/accounts/${name.toUpperCase()}/key`);
      expect(await key.json()).toEqual({ username: name, account: b64(account.publicKey) });
    }
    // The one key package is still there for whoever starts a chat.
    expect((await lookup(name)).devices[0]?.key_package).toBe(b64(keyPackage('only')));
    expect((await get(`/v0/accounts/${username()}/key`)).status).toBe(404);
  });

  it('ignores duplicate key packages and stops at the limit', async () => {
    const { name, device: phone } = await registered([keyPackage('a')]);
    const path = `${devicePath(name, phone.id)}/key-packages`;

    const again = await signedFetch(phone, 'POST', path, {
      key_packages: [b64(keyPackage('a')), b64(keyPackage('b'))],
    });
    expect(await again.json()).toEqual({ key_packages: 2 });

    const many = Array.from({ length: MAX_KEY_PACKAGES }, (_, i) =>
      b64(keyPackage(`m${String(i)}`)),
    );
    const full = await signedFetch(phone, 'POST', path, { key_packages: many });
    expect(await full.json()).toEqual({ key_packages: MAX_KEY_PACKAGES });
  });

  it('refuses requests not signed by a certified device of the account', async () => {
    const { name, account, device: phone } = await registered();
    const path = `${devicePath(name, phone.id)}/key-packages`;
    const body = { key_packages: [b64(keyPackage('x'))] };

    // Another device's key signing with this device's certificate.
    const impostor = { ...(await device(account)), certificate: phone.certificate };
    expect((await signedFetch(impostor, 'POST', path, body)).status).toBe(401);

    // A genuine certificate from a different account.
    const stranger = await device(await keys());
    const foreign = `${devicePath(name, stranger.id)}/key-packages`;
    expect((await signedFetch(stranger, 'POST', foreign, body)).status).toBe(401);

    // A valid device signing for another device id.
    const other = `${devicePath(name, random(16))}/key-packages`;
    expect((await signedFetch(phone, 'POST', other, body)).status).toBe(401);

    // A stale signature.
    const stale = await signedFetch(phone, 'POST', path, body, { timestamp: Date.now() - 600_000 });
    expect(await stale.json()).toEqual({ error: 'stale_request' });

    // An unregistered device of the right account.
    const laptop = await device(account);
    const unknown = await signedFetch(
      laptop,
      'POST',
      `${devicePath(name, laptop.id)}/key-packages`,
      body,
    );
    expect(await unknown.json()).toEqual({ error: 'unknown_device' });
  });

  it('keeps a mailbox with whoever claimed it first', async () => {
    const { name, account, device: phone } = await registered();
    const laptop = { ...(await device(account)), mailbox: phone.mailbox };
    const response = await signedFetch(
      laptop,
      'PUT',
      devicePath(name, laptop.id),
      await registrationBody(laptop),
    );
    expect(response.status).toBe(409);
    expect(await response.json()).toEqual({ error: 'mailbox_taken' });
  });

  it(`allows at most ${String(MAX_DEVICES)} devices`, async () => {
    const { name, account } = await registered();
    for (let i = 1; i < MAX_DEVICES; i++) {
      const extra = await device(account);
      const ok = await signedFetch(
        extra,
        'PUT',
        devicePath(name, extra.id),
        await registrationBody(extra),
      );
      expect(ok.status).toBe(200);
    }
    const oneTooMany = await device(account);
    const refused = await signedFetch(
      oneTooMany,
      'PUT',
      devicePath(name, oneTooMany.id),
      await registrationBody(oneTooMany),
    );
    expect(await refused.json()).toEqual({ error: 'too_many_devices' });
  });

  it('answers unknown and malformed lookups without revealing anything', async () => {
    const unknown = await get(`/v0/accounts/${username()}`);
    expect(unknown.status).toBe(404);
    expect(await unknown.json()).toEqual({ error: 'unknown_account' });
    expect((await get('/v0/accounts/no')).status).toBe(400);
  });

  it('refuses registration for an account that does not exist', async () => {
    const ghost = await device(await keys());
    const response = await signedFetch(
      ghost,
      'PUT',
      devicePath(username(), ghost.id),
      await registrationBody(ghost),
    );
    expect(response.status).toBe(404);
  });
});
