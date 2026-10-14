/**
 * Cryptographic checks. The service never holds a user's secret: it only
 * verifies signatures, hashes and proofs of work.
 */
import { concat, u64, utf8 } from './bytes';

/** SHA-256 of `data`. */
export async function sha256(data: Uint8Array): Promise<Uint8Array> {
  return new Uint8Array(await crypto.subtle.digest('SHA-256', data));
}

/** Whether `signature` is a valid Ed25519 signature of `message` by `publicKey`. */
export async function verifyEd25519(
  publicKey: Uint8Array,
  message: Uint8Array,
  signature: Uint8Array,
): Promise<boolean> {
  if (publicKey.length !== 32 || signature.length !== 64) {
    return false;
  }
  try {
    const key = await crypto.subtle.importKey('raw', publicKey, { name: 'Ed25519' }, false, [
      'verify',
    ]);
    return await crypto.subtle.verify({ name: 'Ed25519' }, key, signature, message);
  } catch {
    // Not a valid point on the curve.
    return false;
  }
}

/** Constant-time equality of two byte strings of equal length. */
export function sameBytes(a: Uint8Array, b: Uint8Array): boolean {
  return a.length === b.length && crypto.subtle.timingSafeEqual(a, b);
}

/** Number of leading zero bits of `bytes`. */
export function leadingZeroBits(bytes: Uint8Array): number {
  let bits = 0;
  for (const byte of bytes) {
    if (byte === 0) {
      bits += 8;
      continue;
    }
    return bits + Math.clz32(byte) - 24;
  }
  return bits;
}

/** HMAC-SHA256 of `data` under `key`. */
export async function hmacSha256(key: Uint8Array, data: Uint8Array): Promise<Uint8Array> {
  const imported = await crypto.subtle.importKey(
    'raw',
    key,
    { name: 'HMAC', hash: 'SHA-256' },
    false,
    ['sign'],
  );
  return new Uint8Array(await crypto.subtle.sign('HMAC', imported, data));
}

/** Milliseconds a signed timestamp may differ from the server clock. */
export const CLOCK_WINDOW_MS = 5 * 60 * 1000;

/** Whether `timestamp` (UTC ms) is close enough to `now`. */
export function fresh(timestamp: number, now: number): boolean {
  return Number.isSafeInteger(timestamp) && Math.abs(now - timestamp) <= CLOCK_WINDOW_MS;
}

/** The payload a device signs for a request (`protocol/directory.md`). */
export async function requestPayload(
  method: string,
  path: string,
  timestamp: number,
  body: Uint8Array,
): Promise<Uint8Array> {
  return concat(
    utf8('zelamar/request/v0'),
    utf8(method),
    new Uint8Array([0]),
    utf8(path),
    new Uint8Array([0]),
    u64(timestamp),
    await sha256(body),
  );
}
