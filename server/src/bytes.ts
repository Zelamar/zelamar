/**
 * Byte helpers shared by every module: base64url, big-endian integers,
 * concatenation. Wire formats are specified in `protocol/`.
 */

const encoder = new TextEncoder();

/** UTF-8 bytes of `text`. */
export function utf8(text: string): Uint8Array {
  return encoder.encode(text);
}

/** Concatenates byte arrays into a new one. */
export function concat(...parts: Uint8Array[]): Uint8Array {
  const out = new Uint8Array(parts.reduce((size, part) => size + part.length, 0));
  let offset = 0;
  for (const part of parts) {
    out.set(part, offset);
    offset += part.length;
  }
  return out;
}

/** `value` as 8 bytes, big-endian. */
export function u64(value: number): Uint8Array {
  const out = new Uint8Array(8);
  new DataView(out.buffer).setBigUint64(0, BigInt(value));
  return out;
}

/** `value` as 4 bytes, big-endian. */
export function u32(value: number): Uint8Array {
  const out = new Uint8Array(4);
  new DataView(out.buffer).setUint32(0, value);
  return out;
}

/** A standalone `ArrayBuffer` holding exactly `bytes` (for storage and RPC). */
export function buffer(bytes: Uint8Array): ArrayBuffer {
  return bytes.slice().buffer;
}

/** base64url without padding. */
export function toBase64Url(bytes: Uint8Array): string {
  let binary = '';
  for (const byte of bytes) {
    binary += String.fromCharCode(byte);
  }
  return btoa(binary).replaceAll('+', '-').replaceAll('/', '_').replace(/=+$/u, '');
}

/**
 * Decodes base64url without padding. Returns `null` for anything that is
 * not the canonical encoding of some bytes, so one value has one spelling.
 */
export function fromBase64Url(text: string): Uint8Array | null {
  if (!/^[A-Za-z0-9_-]*$/u.test(text) || text.length % 4 === 1) {
    return null;
  }
  const padded = text.replaceAll('-', '+').replaceAll('_', '/');
  let binary: string;
  try {
    binary = atob(padded + '='.repeat((4 - (padded.length % 4)) % 4));
  } catch {
    return null;
  }
  const bytes = Uint8Array.from(binary, (char) => char.charCodeAt(0));
  return toBase64Url(bytes) === text ? bytes : null;
}

/** Decodes base64url of exactly `length` bytes, or `null`. */
export function fixedBase64Url(text: unknown, length: number): Uint8Array | null {
  if (typeof text !== 'string') {
    return null;
  }
  const bytes = fromBase64Url(text);
  return bytes?.length === length ? bytes : null;
}
