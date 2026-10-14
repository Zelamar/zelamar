/**
 * One `@username` (`protocol/directory.md`): the account key, its devices'
 * certificates and mailboxes, and their key packages. The object's name is
 * the username, so a name has exactly one home and claims cannot race.
 */
import { DurableObject } from 'cloudflare:workers';

import { buffer, fixedBase64Url, fromBase64Url, toBase64Url } from './bytes';
import { decodeCertificate, verifyCertificate } from './certificate';
import { fresh, requestPayload, sameBytes, sha256, verifyEd25519 } from './crypto';
import { failure, parseObject, type Reply } from './http';

/** Most devices one account may register. */
export const MAX_DEVICES = 5;
/** Most one-time key packages stored per device. */
export const MAX_KEY_PACKAGES = 100;
/** Largest key package accepted. */
export const MAX_KEY_PACKAGE_BYTES = 16 * 1024;

export type AccountState = 'free' | 'same' | 'taken';
export type ClaimOutcome = 'created' | 'same' | 'taken';

/** The parts of a signed device request the account needs to check it. */
export interface SignedRequest {
  method: string;
  path: string;
  certificate: string | null;
  timestamp: string | null;
  signature: string | null;
  body: ArrayBuffer;
}

export class Account extends DurableObject<Env> {
  /** Whether the name is free, already held by `key`, or held by someone else. */
  state(key: ArrayBuffer): AccountState {
    const current = this.accountKey();
    if (current === null) {
      return 'free';
    }
    return sameBytes(current, new Uint8Array(key)) ? 'same' : 'taken';
  }

  /** Claims the name for `key`. */
  claim(username: string, key: ArrayBuffer): ClaimOutcome {
    const state = this.state(key);
    if (state !== 'free') {
      return state;
    }
    this.createSchema();
    this.ctx.storage.sql.exec(
      'INSERT INTO account (id, username, key, created) VALUES (1, ?, ?, ?)',
      username,
      key,
      Date.now(),
    );
    return 'created';
  }

  /** Registers (or re-registers) a device with its mailbox and key packages. */
  async registerDevice(device: string, request: SignedRequest): Promise<Reply> {
    const auth = await this.authenticate(device, request);
    if ('status' in auth) {
      return auth;
    }
    const body = parseObject(new Uint8Array(request.body));
    const mailbox = fixedBase64Url(body?.mailbox, 32);
    const secretHash = fixedBase64Url(body?.mailbox_secret_hash, 32);
    const keyPackages = parseKeyPackages(body?.key_packages);
    const lastResort = parseKeyPackage(body?.last_resort);
    if (mailbox === null || secretHash === null || keyPackages === null || lastResort === null) {
      return failure(400, 'malformed');
    }
    const hashed = await withHashes(keyPackages);

    const claimed = await this.env.MAILBOX.getByName(toBase64Url(mailbox)).claim(
      buffer(secretHash),
    );
    if (claimed === 'taken') {
      return failure(409, 'mailbox_taken');
    }

    // No await from here on: the checks and writes below are atomic.
    const sql = this.ctx.storage.sql;
    const known = sql.exec('SELECT 1 FROM devices WHERE device = ?', buffer(auth.device)).toArray();
    const devices = sql.exec<{ n: number }>('SELECT COUNT(*) AS n FROM devices').one().n;
    if (known.length === 0 && devices >= MAX_DEVICES) {
      return failure(409, 'too_many_devices');
    }
    sql.exec(
      `INSERT INTO devices (device, certificate, mailbox, last_resort, registered)
       VALUES (?, ?, ?, ?, ?)
       ON CONFLICT (device) DO UPDATE SET
         certificate = excluded.certificate,
         mailbox = excluded.mailbox,
         last_resort = excluded.last_resort`,
      buffer(auth.device),
      buffer(auth.certificate),
      buffer(mailbox),
      buffer(lastResort),
      Date.now(),
    );
    return { status: 200, body: { key_packages: this.store(auth.device, hashed) } };
  }

  /** Adds one-time key packages (and optionally a new last-resort one). */
  async publishKeyPackages(device: string, request: SignedRequest): Promise<Reply> {
    const auth = await this.authenticate(device, request);
    if ('status' in auth) {
      return auth;
    }
    const body = parseObject(new Uint8Array(request.body));
    const keyPackages = parseKeyPackages(body?.key_packages);
    const lastResort =
      body?.last_resort === undefined ? undefined : parseKeyPackage(body.last_resort);
    if (keyPackages === null || lastResort === null) {
      return failure(400, 'malformed');
    }
    const hashed = await withHashes(keyPackages);
    // No await from here on: the checks and writes below are atomic.
    const sql = this.ctx.storage.sql;
    if (
      sql.exec('SELECT 1 FROM devices WHERE device = ?', buffer(auth.device)).toArray().length === 0
    ) {
      return failure(404, 'unknown_device');
    }
    if (lastResort !== undefined) {
      sql.exec(
        'UPDATE devices SET last_resort = ? WHERE device = ?',
        buffer(lastResort),
        buffer(auth.device),
      );
    }
    return { status: 200, body: { key_packages: this.store(auth.device, hashed) } };
  }

  /** The account key and one key package per device; one-time ones are consumed. */
  lookup(): Reply {
    const key = this.accountKey();
    if (key === null) {
      return failure(404, 'unknown_account');
    }
    const sql = this.ctx.storage.sql;
    const devices = sql
      .exec<{ device: ArrayBuffer; certificate: ArrayBuffer; last_resort: ArrayBuffer }>(
        'SELECT device, certificate, last_resort FROM devices ORDER BY registered, device',
      )
      .toArray();
    const found = devices.map((row) => {
      const oneTime = sql
        .exec<{ key_package: ArrayBuffer }>(
          `DELETE FROM key_packages WHERE seq = (
             SELECT seq FROM key_packages WHERE device = ? ORDER BY seq LIMIT 1
           ) RETURNING key_package`,
          row.device,
        )
        .toArray();
      const keyPackage = oneTime[0]?.key_package ?? row.last_resort;
      return {
        certificate: toBase64Url(new Uint8Array(row.certificate)),
        key_package: toBase64Url(new Uint8Array(keyPackage)),
      };
    });
    return { status: 200, body: { account: toBase64Url(key), devices: found } };
  }

  /** Checks a signed device request; on success returns the device and its certificate. */
  private async authenticate(
    device: string,
    request: SignedRequest,
  ): Promise<{ device: Uint8Array; certificate: Uint8Array } | Reply> {
    const key = this.accountKey();
    if (key === null) {
      return failure(404, 'unknown_account');
    }
    const deviceId = fixedBase64Url(device, 16);
    const certificateBytes = fromBase64Url(request.certificate ?? '');
    const certificate = certificateBytes === null ? null : decodeCertificate(certificateBytes);
    const signature = fixedBase64Url(request.signature, 64);
    if (
      deviceId === null ||
      certificateBytes === null ||
      certificate === null ||
      signature === null ||
      !sameBytes(certificate.account, key) ||
      !sameBytes(certificate.device, deviceId) ||
      !(await verifyCertificate(certificate))
    ) {
      return failure(401, 'unauthorized');
    }
    const timestamp = /^\d{1,16}$/u.test(request.timestamp ?? '') ? Number(request.timestamp) : NaN;
    if (!fresh(timestamp, Date.now())) {
      return failure(400, 'stale_request');
    }
    const payload = await requestPayload(
      request.method,
      request.path,
      timestamp,
      new Uint8Array(request.body),
    );
    if (!(await verifyEd25519(certificate.deviceKey, payload, signature))) {
      return failure(401, 'unauthorized');
    }
    return { device: deviceId, certificate: certificateBytes };
  }

  /** Stores new key packages up to the limit; returns how many the device has. */
  private store(device: Uint8Array, keyPackages: [Uint8Array, Uint8Array][]): number {
    const sql = this.ctx.storage.sql;
    const count = (): number =>
      sql
        .exec<{ n: number }>(
          'SELECT COUNT(*) AS n FROM key_packages WHERE device = ?',
          buffer(device),
        )
        .one().n;
    let stored = count();
    for (const [hash, keyPackage] of keyPackages) {
      if (stored >= MAX_KEY_PACKAGES) {
        break;
      }
      const inserted = sql
        .exec(
          `INSERT INTO key_packages (device, hash, key_package) VALUES (?, ?, ?)
           ON CONFLICT (hash) DO NOTHING RETURNING seq`,
          buffer(device),
          buffer(hash),
          buffer(keyPackage),
        )
        .toArray().length;
      stored += inserted;
    }
    return stored;
  }

  private accountKey(): Uint8Array | null {
    const exists = this.ctx.storage.sql
      .exec("SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'account'")
      .toArray().length;
    if (exists === 0) {
      return null;
    }
    const rows = this.ctx.storage.sql
      .exec<{ key: ArrayBuffer }>('SELECT key FROM account WHERE id = 1')
      .toArray();
    return rows[0] === undefined ? null : new Uint8Array(rows[0].key);
  }

  private createSchema(): void {
    this.ctx.storage.sql.exec(`
      CREATE TABLE IF NOT EXISTS account (
        id INTEGER PRIMARY KEY CHECK (id = 1),
        username TEXT NOT NULL,
        key BLOB NOT NULL,
        created INTEGER NOT NULL
      );
      CREATE TABLE IF NOT EXISTS devices (
        device BLOB PRIMARY KEY,
        certificate BLOB NOT NULL,
        mailbox BLOB NOT NULL,
        last_resort BLOB NOT NULL,
        registered INTEGER NOT NULL
      );
      CREATE TABLE IF NOT EXISTS key_packages (
        seq INTEGER PRIMARY KEY AUTOINCREMENT,
        device BLOB NOT NULL,
        hash BLOB NOT NULL UNIQUE,
        key_package BLOB NOT NULL
      );
      CREATE INDEX IF NOT EXISTS key_packages_by_device ON key_packages (device, seq);
    `);
  }
}

function parseKeyPackage(value: unknown): Uint8Array | null {
  if (typeof value !== 'string') {
    return null;
  }
  const bytes = fromBase64Url(value);
  return bytes !== null && bytes.length > 0 && bytes.length <= MAX_KEY_PACKAGE_BYTES ? bytes : null;
}

/** A list of key packages, or `null` if it or any of them is malformed. */
function parseKeyPackages(value: unknown): Uint8Array[] | null {
  if (!Array.isArray(value) || value.length > MAX_KEY_PACKAGES) {
    return null;
  }
  const parsed: Uint8Array[] = [];
  for (const item of value) {
    const keyPackage = parseKeyPackage(item);
    if (keyPackage === null) {
      return null;
    }
    parsed.push(keyPackage);
  }
  return parsed;
}

/** Pairs each key package with its SHA-256, the de-duplication key. */
async function withHashes(keyPackages: Uint8Array[]): Promise<[Uint8Array, Uint8Array][]> {
  return Promise.all(
    keyPackages.map(async (keyPackage): Promise<[Uint8Array, Uint8Array]> => [
      await sha256(keyPackage),
      keyPackage,
    ]),
  );
}

export class Invites extends DurableObject<Env> {
  /** Marks `serial` as used; `false` if it already was. */
  redeem(serial: number): boolean {
    const sql = this.ctx.storage.sql;
    sql.exec(
      'CREATE TABLE IF NOT EXISTS redeemed (serial INTEGER PRIMARY KEY, at INTEGER NOT NULL)',
    );
    return (
      sql
        .exec(
          'INSERT INTO redeemed (serial, at) VALUES (?, ?) ON CONFLICT DO NOTHING RETURNING serial',
          serial,
          Date.now(),
        )
        .toArray().length === 1
    );
  }
}
