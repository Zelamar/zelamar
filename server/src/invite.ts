/**
 * Closed-beta invite codes. A code is `serial (4) || mac (8)`, where `mac`
 * is the first 8 bytes of HMAC-SHA256(INVITE_KEY, "zelamar/invite/v0" ||
 * serial), written in base32 as `xxxxx-xxxxx-xxxxx-xxxxx`. The key never
 * leaves the service and its owner, so codes need no admin API; the
 * `Invites` object makes each serial single-use.
 */
import { concat, u32, utf8 } from './bytes';
import { hmacSha256, sameBytes } from './crypto';

const ALPHABET = 'abcdefghijklmnopqrstuvwxyz234567';
const CODE_BYTES = 12;
const CONTEXT = utf8('zelamar/invite/v0');

/** RFC 4648 base32, lowercase, no padding. */
export function toBase32(bytes: Uint8Array): string {
  let out = '';
  let value = 0;
  let bits = 0;
  for (const byte of bytes) {
    value = ((value << 8) | byte) & 0xfff;
    bits += 8;
    while (bits >= 5) {
      out += ALPHABET.charAt((value >>> (bits - 5)) & 31);
      bits -= 5;
    }
  }
  if (bits > 0) {
    out += ALPHABET.charAt((value << (5 - bits)) & 31);
  }
  return out;
}

/** Decodes canonical lowercase base32 without padding, or `null`. */
export function fromBase32(text: string): Uint8Array | null {
  const out: number[] = [];
  let value = 0;
  let bits = 0;
  for (const char of text) {
    const digit = ALPHABET.indexOf(char);
    if (digit < 0) {
      return null;
    }
    value = ((value << 5) | digit) & 0xfff;
    bits += 5;
    if (bits >= 8) {
      out.push((value >>> (bits - 8)) & 0xff);
      bits -= 8;
    }
  }
  // Leftover bits must be zero padding, so each code has one spelling.
  if (bits >= 5 || (value & ((1 << bits) - 1)) !== 0) {
    return null;
  }
  return Uint8Array.from(out);
}

/** Parses the hex `INVITE_KEY`; `null` if it is not at least 32 bytes. */
export function inviteKey(hex: string): Uint8Array | null {
  if (!/^(?:[0-9a-f]{2}){32,}$/u.test(hex)) {
    return null;
  }
  return Uint8Array.from(hex.match(/../gu) ?? [], (pair) => parseInt(pair, 16));
}

async function mac(key: Uint8Array, serial: number): Promise<Uint8Array> {
  return (await hmacSha256(key, concat(CONTEXT, u32(serial)))).subarray(0, 8);
}

/** The invite code for `serial`. */
export async function makeInvite(key: Uint8Array, serial: number): Promise<string> {
  const code = toBase32(concat(u32(serial), await mac(key, serial)));
  return code.match(/.{1,5}/gu)?.join('-') ?? code;
}

/** The serial of a genuine invite `code`, or `null`. Case, dashes and spaces are ignored. */
export async function inviteSerial(key: Uint8Array, code: unknown): Promise<number | null> {
  if (typeof code !== 'string' || code.length > 64) {
    return null;
  }
  const bytes = fromBase32(code.toLowerCase().replace(/[\s-]/gu, ''));
  if (bytes?.length !== CODE_BYTES) {
    return null;
  }
  const serial = new DataView(bytes.buffer).getUint32(0);
  return sameBytes(bytes.subarray(4), await mac(key, serial)) ? serial : null;
}
