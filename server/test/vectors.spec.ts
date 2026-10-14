/**
 * The core (Rust) writes protocol/vectors/v0.json; these tests run it through
 * the very functions the service uses, so core and server cannot drift apart.
 */
import { describe, expect, it } from 'vitest';

import vectors from '../../protocol/vectors/v0.json';
import { buffer, concat, fromBase64Url } from '../src/bytes';
import { decodeCertificate } from '../src/certificate';
import { checkClaim } from '../src/claim';
import { decodeDelivery, encodeEnvelope } from '../src/frames';
import { checkSignedRequest, type SignedRequest } from '../src/signed';
import { normalizeUsername } from '../src/username';

function bytes(text: string): Uint8Array {
  const decoded = fromBase64Url(text);
  if (decoded === null) {
    throw new Error(`not base64url: ${text}`);
  }
  return decoded;
}

describe('vectors written by the core', () => {
  it('agree on every username rule', () => {
    for (const [input, expected] of vectors.usernames) {
      expect(normalizeUsername(input), String(input)).toBe(expected);
    }
  });

  it('give a claim the service accepts, and only as signed', async () => {
    const { body, work_bits: workBits, timestamp } = vectors.claim;
    const accepted = await checkClaim(body, workBits, timestamp);
    expect(accepted).toMatchObject({ username: 'alice_92', account: bytes(body.account) });

    const replayed = await checkClaim({ ...body, timestamp: timestamp + 1 }, workBits, timestamp);
    expect(replayed).toEqual({ status: 401, body: { error: 'bad_signature' } });
    const harder = await checkClaim(body, 32, timestamp);
    expect(harder).toEqual({ status: 400, body: { error: 'insufficient_work' } });
  });

  it('give a signed request the service accepts, and only untouched', async () => {
    const vector = vectors.request;
    const request: SignedRequest = {
      method: vector.method,
      path: vector.path,
      certificate: vector.headers['zelamar-certificate'],
      timestamp: vector.headers['zelamar-timestamp'],
      signature: vector.headers['zelamar-signature'],
      body: buffer(bytes(vector.body)),
    };
    const certificate = decodeCertificate(bytes(request.certificate ?? ''));
    expect(certificate).not.toBeNull();
    const account = certificate?.account ?? new Uint8Array(32);
    const device = vector.path.split('/')[5] ?? '';

    const accepted = await checkSignedRequest(request, account, device, vector.timestamp);
    expect(accepted).toMatchObject({ device: certificate?.device });

    const tampered = { ...request, body: buffer(concat(bytes(vector.body), Uint8Array.of(0x20))) };
    expect(await checkSignedRequest(tampered, account, device, vector.timestamp)).toEqual({
      status: 401,
      body: { error: 'unauthorized' },
    });
    const late = await checkSignedRequest(request, account, device, vector.timestamp + 600_000);
    expect(late).toEqual({ status: 400, body: { error: 'stale_request' } });
  });

  it('give a delivery frame the service reads as intended', () => {
    const vector = vectors.delivery;
    expect(decodeDelivery(bytes(vector.frame))).toEqual({
      id: bytes(vector.id),
      recipients: vector.recipients.map(bytes),
      payload: bytes(vector.payload),
    });
  });

  it('encode envelopes exactly as the service does', () => {
    const vector = vectors.envelope;
    const encoded = encodeEnvelope(bytes(vector.id), bytes(vector.mailbox), bytes(vector.payload));
    expect(encoded).toEqual(bytes(vector.encoded));
  });

  it('frame acknowledgements as the mailbox expects', () => {
    const vector = vectors.acknowledgement;
    expect(bytes(vector.frame)).toEqual(concat(Uint8Array.of(1), ...vector.ids.map(bytes)));
  });
});
