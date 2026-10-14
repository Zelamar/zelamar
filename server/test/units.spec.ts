import { describe, expect, it } from 'vitest';

import { concat, fromBase64Url, toBase64Url, utf8 } from '../src/bytes';
import { decodeCertificate, verifyCertificate } from '../src/certificate';
import { leadingZeroBits } from '../src/crypto';
import { decodeDelivery, encodeDelivery, MAX_PAYLOAD } from '../src/frames';
import { fromBase32, inviteKey, inviteSerial, makeInvite, toBase32 } from '../src/invite';
import { isReserved, normalizeUsername } from '../src/username';
import { device, keys, random } from './helpers';

describe('base64url', () => {
  it('round-trips and accepts only the canonical spelling', () => {
    for (const length of [0, 1, 2, 3, 31, 32, 33]) {
      const bytes = random(length);
      expect(fromBase64Url(toBase64Url(bytes))).toEqual(bytes);
    }
    expect(fromBase64Url('AA==')).toBeNull(); // padding
    expect(fromBase64Url('AB')).toBeNull(); // non-zero spare bits
    expect(fromBase64Url('A')).toBeNull(); // impossible length
    expect(fromBase64Url('a+b/')).toBeNull(); // standard alphabet
  });
});

describe('invite codes', () => {
  const key = inviteKey('00'.repeat(32));

  it('round-trip base32 and refuse non-canonical text', () => {
    const bytes = random(12);
    expect(fromBase32(toBase32(bytes))).toEqual(bytes);
    expect(fromBase32('aaaab')).toBeNull(); // non-zero padding bits
    expect(fromBase32('abc1')).toBeNull(); // outside the alphabet
  });

  it('verify their own serial, in any case and spacing, and nothing else', async () => {
    if (key === null) {
      throw new Error('test key');
    }
    const code = await makeInvite(key, 41);
    expect(code).toMatch(/^[a-z2-7]{5}-[a-z2-7]{5}-[a-z2-7]{5}-[a-z2-7]{5}$/u);
    expect(await inviteSerial(key, code)).toBe(41);
    expect(await inviteSerial(key, ` ${code.toUpperCase().replaceAll('-', ' ')} `)).toBe(41);

    const tampered = (code.startsWith('a') ? 'b' : 'a') + code.slice(1);
    expect(await inviteSerial(key, tampered)).toBeNull();
    const other = inviteKey('01'.repeat(32));
    expect(other === null ? null : await inviteSerial(other, code)).toBeNull();
  });

  it('match the vector shared with scripts/invites.mjs', async () => {
    expect(key === null ? null : await makeInvite(key, 1)).toBe('aaaaa-anpo2-yv3df-cwzmq');
  });

  it('demand a key of at least 32 bytes in hex', () => {
    expect(inviteKey('00'.repeat(31))).toBeNull();
    expect(inviteKey('zz'.repeat(32))).toBeNull();
  });
});

describe('usernames', () => {
  it('are lowercased and checked', () => {
    expect(normalizeUsername('Alice_92')).toBe('alice_92');
    expect(normalizeUsername('a_b_c')).toBe('a_b_c');
    for (const bad of ['', 'ab', '_abc', '9abc', 'abc_', 'a__b', 'ab.c', 'añb', 42, null]) {
      expect(normalizeUsername(bad)).toBeNull();
    }
    expect(isReserved('support')).toBe(true);
    expect(isReserved('zelamar_help')).toBe(true);
    expect(isReserved('ada')).toBe(false);
  });
});

describe('proof of work', () => {
  it('counts leading zero bits', () => {
    expect(leadingZeroBits(Uint8Array.of(0x80))).toBe(0);
    expect(leadingZeroBits(Uint8Array.of(0x00, 0x01))).toBe(15);
    expect(leadingZeroBits(Uint8Array.of(0x00, 0x00))).toBe(16);
  });
});

describe('delivery frames', () => {
  it('round-trip', () => {
    const delivery = { id: random(16), recipients: [random(32), random(32)], payload: random(100) };
    const decoded = decodeDelivery(encodeDelivery(delivery));
    expect(decoded).toEqual(delivery);
  });

  it('accept the largest payload and refuse a larger one', () => {
    const largest = encodeDelivery({
      id: random(16),
      recipients: [random(32)],
      payload: new Uint8Array(MAX_PAYLOAD),
    });
    expect(typeof decodeDelivery(largest)).toBe('object');
    const larger = encodeDelivery({
      id: random(16),
      recipients: [random(32)],
      payload: new Uint8Array(MAX_PAYLOAD + 1),
    });
    expect(decodeDelivery(larger)).toBe('too_large');
  });
});

describe('device certificates', () => {
  it('verify only when untouched', async () => {
    const phone = await device(await keys());
    const certificate = decodeCertificate(phone.certificate);
    expect(certificate).not.toBeNull();
    if (certificate === null) {
      return;
    }
    expect(await verifyCertificate(certificate)).toBe(true);

    const flipped = phone.certificate.slice();
    flipped[40] = (flipped[40] ?? 0) ^ 1; // inside the device id
    const forged = decodeCertificate(flipped);
    expect(forged === null ? false : await verifyCertificate(forged)).toBe(false);
  });

  it('refuse other versions, truncation and trailing bytes', async () => {
    const phone = await device(await keys());
    const otherVersion = phone.certificate.slice();
    otherVersion[0] = 1;
    expect(decodeCertificate(otherVersion)).toBeNull();
    expect(decodeCertificate(phone.certificate.subarray(0, 100))).toBeNull();
    expect(decodeCertificate(concat(phone.certificate, utf8('x')))).toBeNull();
  });
});
