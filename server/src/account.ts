/**
 * One `@username` (`protocol/directory.md`): the account key, its devices'
 * certificates and mailboxes, and their key packages. The object's name is
 * the username, so a name has exactly one home and claims cannot race.
 */
import { DurableObject } from 'cloudflare:workers';

import { buffer, fixedBase64Url, fromBase64Url, toBase64Url } from './bytes';
import { sameBytes, sha256 } from './crypto';
import { failure, parseObject, type Reply } from './http';
import {
  checkAccountRequest,
  checkSignedRequest,
  type SignedBy,
  type SignedRequest,
} from './signed';

/** Most devices one account may register. */
export const MAX_DEVICES = 5;
/** Most one-time key packages stored per device. */
export const MAX_KEY_PACKAGES = 100;
/** Largest key package accepted. */
export const MAX_KEY_PACKAGE_BYTES = 16 * 1024;
/** Largest sealed recovery kit kept (ADR-0018). */
export const MAX_RECOVERY_KIT = 64 * 1024;
/** Most names one request for device lists may ask about (ADR-0018). */
export const MAX_DIRECTORY_BATCH = 64;

export type AccountState = 'free' | 'same' | 'taken';
export type ClaimOutcome = 'created' | 'same' | 'taken';

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

  /**
   * Takes a device out of the directory (ADR-0014). The device itself may
   * sign the request, or the account key may (`accountSignature`).
   */
  async removeDevice(
    device: string,
    request: SignedRequest,
    accountSignature: string | null,
  ): Promise<Reply> {
    const key = this.accountKey();
    if (key === null) {
      return failure(404, 'unknown_account');
    }
    const id = fixedBase64Url(device, 16);
    if (id === null) {
      return failure(404, 'unknown_device');
    }
    const auth =
      accountSignature === null
        ? await checkSignedRequest(request, key, device, Date.now())
        : await checkAccountRequest({ ...request, signature: accountSignature }, key, Date.now());
    if (auth !== true && 'status' in auth) {
      return auth;
    }
    // No await from here on: the writes below are atomic.
    const sql = this.ctx.storage.sql;
    sql.exec('DELETE FROM key_packages WHERE device = ?', buffer(id));
    sql.exec('DELETE FROM devices WHERE device = ?', buffer(id));
    const left = sql.exec<{ n: number }>('SELECT COUNT(*) AS n FROM devices').one().n;
    return { status: 200, body: { devices: left } };
  }

  /**
   * Keeps the account's sealed recovery kit and the hash of the token that
   * fetches it (ADR-0018), replacing any earlier one. Only the account key
   * may sign this.
   */
  async saveRecovery(request: SignedRequest): Promise<Reply> {
    const key = this.accountKey();
    if (key === null) {
      return failure(404, 'unknown_account');
    }
    const auth = await checkAccountRequest(request, key, Date.now());
    if (auth !== true) {
      return auth;
    }
    const body = parseObject(new Uint8Array(request.body));
    const tokenHash = fixedBase64Url(body?.token_hash, 32);
    const kit = typeof body?.kit === 'string' ? fromBase64Url(body.kit) : null;
    if (tokenHash === null || kit === null || kit.length === 0) {
      return failure(400, 'malformed');
    }
    if (kit.length > MAX_RECOVERY_KIT) {
      return failure(413, 'too_large');
    }
    this.createSchema();
    this.ctx.storage.sql.exec(
      `INSERT INTO recovery (id, token_hash, kit, updated) VALUES (1, ?, ?, ?)
       ON CONFLICT (id) DO UPDATE SET
         token_hash = excluded.token_hash,
         kit = excluded.kit,
         updated = excluded.updated`,
      buffer(tokenHash),
      buffer(kit),
      Date.now(),
    );
    return { status: 200, body: {} };
  }

  /**
   * The sealed recovery kit, for whoever presents its token. A wrong token,
   * no kit and no account all look the same.
   */
  async openRecovery(token: ArrayBuffer): Promise<Reply> {
    const hash = await sha256(new Uint8Array(token));
    if (this.holder() === null) {
      return failure(403, 'wrong_key');
    }
    this.createSchema();
    const row = this.ctx.storage.sql
      .exec<{ token_hash: ArrayBuffer; kit: ArrayBuffer }>(
        'SELECT token_hash, kit FROM recovery WHERE id = 1',
      )
      .toArray()[0];
    if (row === undefined || !sameBytes(hash, new Uint8Array(row.token_hash))) {
      return failure(403, 'wrong_key');
    }
    return { status: 200, body: { kit: toBase64Url(new Uint8Array(row.kit)) } };
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
    const holder = this.holder();
    if (holder === null) {
      return failure(404, 'unknown_account');
    }
    const devices = this.ctx.storage.sql
      .exec<DeviceRow>(
        'SELECT device, certificate, last_resort FROM devices ORDER BY registered, device',
      )
      .toArray();
    return found(
      holder,
      devices.map((row) => this.withKeyPackage(row)),
    );
  }

  /** One key package of one device, for adding it to a chat (ADR-0014). */
  lookupDevice(device: ArrayBuffer): Reply {
    const holder = this.holder();
    if (holder === null) {
      return failure(404, 'unknown_account');
    }
    const row = this.ctx.storage.sql
      .exec<DeviceRow>(
        'SELECT device, certificate, last_resort FROM devices WHERE device = ?',
        device,
      )
      .toArray()[0];
    return row === undefined
      ? failure(404, 'unknown_device')
      : found(holder, [this.withKeyPackage(row)]);
  }

  /** Every device's certificate, without handing out any key package (ADR-0014). */
  listDevices(): Reply {
    const holder = this.holder();
    if (holder === null) {
      return failure(404, 'unknown_account');
    }
    const devices = this.ctx.storage.sql
      .exec<{ certificate: ArrayBuffer }>(
        'SELECT certificate FROM devices ORDER BY registered, device',
      )
      .toArray();
    return found(
      holder,
      devices.map((row) => ({ certificate: toBase64Url(new Uint8Array(row.certificate)) })),
    );
  }

  /** Only which account holds the name; hands out no key package. */
  keyOf(): Reply {
    const holder = this.holder();
    return holder === null
      ? failure(404, 'unknown_account')
      : { status: 200, body: { username: holder.username, account: toBase64Url(holder.key) } };
  }

  /** A device's certificate and its next key package, which is consumed. */
  private withKeyPackage(row: DeviceRow): Record<string, string> {
    const oneTime = this.ctx.storage.sql
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
  }

  /** Checks a signed device request against this account's key. */
  private async authenticate(device: string, request: SignedRequest): Promise<SignedBy | Reply> {
    const key = this.accountKey();
    if (key === null) {
      return failure(404, 'unknown_account');
    }
    return checkSignedRequest(request, key, device, Date.now());
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
    return this.holder()?.key ?? null;
  }

  /** The name and the account key holding it; `null` while unclaimed. */
  private holder(): { username: string; key: Uint8Array } | null {
    const exists = this.ctx.storage.sql
      .exec("SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'account'")
      .toArray().length;
    if (exists === 0) {
      return null;
    }
    const rows = this.ctx.storage.sql
      .exec<{ username: string; key: ArrayBuffer }>(
        'SELECT username, key FROM account WHERE id = 1',
      )
      .toArray();
    return rows[0] === undefined
      ? null
      : { username: rows[0].username, key: new Uint8Array(rows[0].key) };
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
      CREATE TABLE IF NOT EXISTS recovery (
        id INTEGER PRIMARY KEY CHECK (id = 1),
        token_hash BLOB NOT NULL,
        kit BLOB NOT NULL,
        updated INTEGER NOT NULL
      );
    `);
  }
}

interface DeviceRow extends Record<string, SqlStorageValue> {
  device: ArrayBuffer;
  certificate: ArrayBuffer;
  last_resort: ArrayBuffer;
}

/** A directory answer about `holder` listing `devices`. */
function found(
  holder: { username: string; key: Uint8Array },
  devices: Record<string, string>[],
): Reply {
  return {
    status: 200,
    body: { username: holder.username, account: toBase64Url(holder.key), devices },
  };
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
