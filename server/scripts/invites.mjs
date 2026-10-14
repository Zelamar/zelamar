#!/usr/bin/env node
// Prints closed-beta invite codes (src/invite.ts explains the format).
//
//   INVITE_KEY=<hex> node scripts/invites.mjs <first serial> [count]
//   node scripts/invites.mjs --self-test
//
// Use the same INVITE_KEY as the deployed service (`wrangler secret put
// INVITE_KEY`). Every serial yields one single-use code; keep a note of the
// serials already handed out.
import { Buffer } from 'node:buffer';
import console from 'node:console';
import { createHmac } from 'node:crypto';
import process from 'node:process';

const ALPHABET = 'abcdefghijklmnopqrstuvwxyz234567';

function base32(bytes) {
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

export function invite(keyHex, serial) {
  const serialBytes = Buffer.alloc(4);
  serialBytes.writeUInt32BE(serial);
  const mac = createHmac('sha256', Buffer.from(keyHex, 'hex'))
    .update(Buffer.concat([Buffer.from('zelamar/invite/v0'), serialBytes]))
    .digest()
    .subarray(0, 8);
  return base32(Buffer.concat([serialBytes, mac]))
    .match(/.{1,5}/g)
    .join('-');
}

// The same vector is asserted in test/units.spec.ts against src/invite.ts.
const VECTOR = { key: '00'.repeat(32), serial: 1, code: 'aaaaa-anpo2-yv3df-cwzmq' };

const [first, count = '1'] = process.argv.slice(2);
if (first === '--self-test') {
  const code = invite(VECTOR.key, VECTOR.serial);
  if (code !== VECTOR.code) {
    console.error(`self-test failed: got ${code}`);
    process.exit(1);
  }
  console.log('ok');
} else {
  const key = process.env.INVITE_KEY ?? '';
  if (!/^(?:[0-9a-f]{2}){32,}$/.test(key) || !/^\d+$/.test(first ?? '') || !/^\d+$/.test(count)) {
    console.error(
      'usage: INVITE_KEY=<hex, 32+ bytes> node scripts/invites.mjs <first serial> [count]',
    );
    process.exit(2);
  }
  for (let serial = Number(first); serial < Number(first) + Number(count); serial++) {
    console.log(`${String(serial).padStart(6)}  ${invite(key, serial)}`);
  }
}
