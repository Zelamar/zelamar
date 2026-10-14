/**
 * Checking a username claim (`protocol/directory.md`): every stateless
 * rule, before any Durable Object is touched.
 */
import { concat, fixedBase64Url, u64, utf8 } from './bytes';
import { fresh, leadingZeroBits, sha256, verifyEd25519 } from './crypto';
import { failure, type Reply } from './http';
import { isReserved, normalizeUsername } from './username';

/** A claim that passed every stateless check. */
export interface CheckedClaim {
  username: string;
  account: Uint8Array;
  invite: unknown;
}

/** Checks a claim's fields at time `now` against a difficulty of `workBits`. */
export async function checkClaim(
  fields: Record<string, unknown>,
  workBits: number,
  now: number,
): Promise<CheckedClaim | Reply> {
  const username = normalizeUsername(fields.username);
  if (username === null) {
    return failure(400, 'invalid_username');
  }
  if (isReserved(username)) {
    return failure(403, 'reserved');
  }
  const account = fixedBase64Url(fields.account, 32);
  const work = fixedBase64Url(fields.work, 8);
  const signature = fixedBase64Url(fields.signature, 64);
  const timestamp = fields.timestamp;
  if (account === null || work === null || signature === null || typeof timestamp !== 'number') {
    return failure(400, 'malformed');
  }
  if (!fresh(timestamp, now)) {
    return failure(400, 'stale_request');
  }
  const name = utf8(username);
  const nameField = concat(Uint8Array.of(name.length), name);
  const proof = await sha256(concat(utf8('zelamar/work/v0'), account, nameField, work));
  if (leadingZeroBits(proof) < workBits) {
    return failure(400, 'insufficient_work');
  }
  const payload = concat(utf8('zelamar/claim/v0'), nameField, u64(timestamp), work);
  if (!(await verifyEd25519(account, payload, signature))) {
    return failure(401, 'bad_signature');
  }
  return { username, account, invite: fields.invite };
}
