/**
 * One device's mailbox (`protocol/delivery.md`): a queue of envelopes held
 * until the device acknowledges them, delivered live over hibernating
 * WebSockets. The mailbox stores `SHA-256(secret)` and nothing about whose
 * it is.
 */
import { DurableObject } from 'cloudflare:workers';

import { buffer, fromBase64Url } from './bytes';
import { sameBytes, sha256 } from './crypto';
import { json } from './http';

/** Most envelopes waiting in one mailbox. */
export const MAX_QUEUED = 10_000;
/** How long an envelope waits before it is dropped. */
export const TTL_MS = 30 * 24 * 60 * 60 * 1000;
/** Envelopes sent to one connection and not yet acknowledged, at most. */
export const WINDOW = 64;

/** Bytes of file chunks one mailbox may upload per UTC day (ADR-0016). */
export const UPLOAD_QUOTA = 512 * 1024 * 1024;

/** Acknowledgement frame tag: `0x01` followed by 16-byte envelope ids. */
const ACK = 0x01;

export type ClaimOutcome = 'claimed' | 'same' | 'taken';
export type DepositOutcome = 'queued' | 'duplicate' | 'unknown' | 'full';
export type UploadAllowance = 'allowed' | 'unauthorized' | 'over_quota';

/** Per-connection state that survives hibernation. */
interface Cursor {
  /** Highest `seq` sent on this connection. */
  last: number;
}

export class Mailbox extends DurableObject<Env> {
  /** Cached number of queued envelopes; `null` when unknown. */
  private queued: number | null = null;

  /** Takes ownership of the mailbox for whoever knows the secret behind `secretHash`. */
  claim(secretHash: ArrayBuffer): ClaimOutcome {
    if (secretHash.byteLength !== 32) {
      throw new RangeError('secret hash must be 32 bytes');
    }
    this.createSchema();
    const owner = this.owner();
    if (owner !== null) {
      return sameBytes(owner, new Uint8Array(secretHash)) ? 'same' : 'taken';
    }
    this.ctx.storage.sql.exec('INSERT INTO owner (id, secret_hash) VALUES (1, ?)', secretHash);
    return 'claimed';
  }

  /**
   * Whether the holder of `secret` owns this mailbox and may upload `bytes`
   * more of file chunks today; counts them if so.
   */
  async allowUpload(secret: ArrayBuffer, bytes: number): Promise<UploadAllowance> {
    const owner = this.owner();
    if (
      owner === null ||
      secret.byteLength !== 32 ||
      !sameBytes(await sha256(new Uint8Array(secret)), owner)
    ) {
      return 'unauthorized';
    }
    const sql = this.ctx.storage.sql;
    sql.exec(
      'CREATE TABLE IF NOT EXISTS uploads (id INTEGER PRIMARY KEY CHECK (id = 1), day INTEGER NOT NULL, bytes INTEGER NOT NULL)',
    );
    const today = Math.floor(Date.now() / 86_400_000);
    const row = sql
      .exec<{ day: number; bytes: number }>('SELECT day, bytes FROM uploads WHERE id = 1')
      .toArray()[0];
    const used = row?.day === today ? row.bytes : 0;
    if (used + bytes > UPLOAD_QUOTA) {
      return 'over_quota';
    }
    sql.exec(
      `INSERT INTO uploads (id, day, bytes) VALUES (1, ?, ?)
       ON CONFLICT (id) DO UPDATE SET day = excluded.day, bytes = excluded.bytes`,
      today,
      used + bytes,
    );
    return 'allowed';
  }

  /** Queues one envelope and pushes it to live connections. */
  async deposit(id: ArrayBuffer, envelope: ArrayBuffer): Promise<DepositOutcome> {
    if (this.owner() === null) {
      return 'unknown';
    }
    const queued = this.count();
    if (queued >= MAX_QUEUED) {
      return 'full';
    }
    const expires = Date.now() + TTL_MS;
    const inserted = this.ctx.storage.sql
      .exec(
        `INSERT INTO envelopes (id, envelope, expires) VALUES (?, ?, ?)
         ON CONFLICT (id) DO NOTHING RETURNING seq`,
        id,
        envelope,
        expires,
      )
      .toArray();
    if (inserted.length === 0) {
      return 'duplicate';
    }
    this.queued = queued + 1;
    for (const socket of this.ctx.getWebSockets()) {
      this.pump(socket);
    }
    if ((await this.ctx.storage.getAlarm()) === null) {
      await this.ctx.storage.setAlarm(expires);
    }
    return 'queued';
  }

  /** Opens a live connection for the mailbox's owner. */
  override async fetch(request: Request): Promise<Response> {
    if (request.headers.get('upgrade')?.toLowerCase() !== 'websocket') {
      return json({ error: 'upgrade_required' }, 426);
    }
    const owner = this.owner();
    if (owner === null) {
      return json({ error: 'unknown_mailbox' }, 404);
    }
    const secret = /^Bearer ([A-Za-z0-9_-]+)$/u.exec(request.headers.get('authorization') ?? '');
    const bytes = secret?.[1] === undefined ? null : fromBase64Url(secret[1]);
    if (bytes?.length !== 32 || !sameBytes(await sha256(bytes), owner)) {
      return json({ error: 'unauthorized' }, 401);
    }

    const [client, server] = Object.values(new WebSocketPair()) as [WebSocket, WebSocket];
    this.ctx.acceptWebSocket(server);
    server.serializeAttachment({ last: 0 } satisfies Cursor);
    this.pump(server);
    return new Response(null, { status: 101, webSocket: client });
  }

  override webSocketMessage(socket: WebSocket, message: string | ArrayBuffer): void {
    const bytes = typeof message === 'string' ? null : new Uint8Array(message);
    if (bytes?.[0] !== ACK || bytes.length < 17 || (bytes.length - 1) % 16 !== 0) {
      socket.close(1008, 'expected an acknowledgement frame');
      return;
    }
    for (let at = 1; at < bytes.length; at += 16) {
      this.ctx.storage.sql.exec(
        'DELETE FROM envelopes WHERE id = ?',
        buffer(bytes.subarray(at, at + 16)),
      );
    }
    this.queued = null;
    for (const other of this.ctx.getWebSockets()) {
      this.pump(other);
    }
  }

  override webSocketClose(socket: WebSocket, code: number): void {
    // Complete the closing handshake. 1005 and 1006 only describe a close;
    // they may not be sent, and the socket may already be closed.
    try {
      socket.close(code === 1005 || code === 1006 ? 1000 : code, 'closing');
    } catch {
      // Already closed.
    }
  }

  /** Drops expired envelopes; wakes again for the next expiry. */
  override async alarm(): Promise<void> {
    if (this.owner() === null) {
      return;
    }
    this.ctx.storage.sql.exec('DELETE FROM envelopes WHERE expires <= ?', Date.now());
    this.queued = null;
    const next = this.ctx.storage.sql
      .exec<{ expires: number }>('SELECT MIN(expires) AS expires FROM envelopes')
      .one().expires;
    if (typeof next === 'number') {
      await this.ctx.storage.setAlarm(next);
    }
  }

  /** Sends queued envelopes to `socket` until its window is full. */
  private pump(socket: WebSocket): void {
    const cursor = socket.deserializeAttachment() as Cursor | null;
    if (cursor === null) {
      return;
    }
    const sql = this.ctx.storage.sql;
    const inFlight = sql
      .exec<{ n: number }>('SELECT COUNT(*) AS n FROM envelopes WHERE seq <= ?', cursor.last)
      .one().n;
    const room = WINDOW - inFlight;
    if (room <= 0) {
      return;
    }
    const rows = sql.exec<{ seq: number; envelope: ArrayBuffer }>(
      'SELECT seq, envelope FROM envelopes WHERE seq > ? ORDER BY seq LIMIT ?',
      cursor.last,
      room,
    );
    let last = cursor.last;
    for (const row of rows) {
      try {
        socket.send(row.envelope);
      } catch {
        // The connection is closing; its envelopes stay queued for the next one.
        break;
      }
      last = row.seq;
    }
    if (last !== cursor.last) {
      socket.serializeAttachment({ last } satisfies Cursor);
    }
  }

  /** The owner's secret hash; `null` while unclaimed (and nothing is stored). */
  private owner(): Uint8Array | null {
    const exists = this.ctx.storage.sql
      .exec("SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'owner'")
      .toArray().length;
    if (exists === 0) {
      return null;
    }
    const rows = this.ctx.storage.sql
      .exec<{ secret_hash: ArrayBuffer }>('SELECT secret_hash FROM owner WHERE id = 1')
      .toArray();
    return rows[0] === undefined ? null : new Uint8Array(rows[0].secret_hash);
  }

  private count(): number {
    this.queued ??= this.ctx.storage.sql
      .exec<{ n: number }>('SELECT COUNT(*) AS n FROM envelopes')
      .one().n;
    return this.queued;
  }

  private createSchema(): void {
    this.ctx.storage.sql.exec(`
      CREATE TABLE IF NOT EXISTS owner (
        id INTEGER PRIMARY KEY CHECK (id = 1),
        secret_hash BLOB NOT NULL
      );
      CREATE TABLE IF NOT EXISTS envelopes (
        seq INTEGER PRIMARY KEY AUTOINCREMENT,
        id BLOB NOT NULL UNIQUE,
        envelope BLOB NOT NULL,
        expires INTEGER NOT NULL
      );
    `);
  }
}
