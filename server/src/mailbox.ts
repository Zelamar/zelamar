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

/** At most one push wake-up per mailbox in this long (ADR-0017). */
export const PUSH_INTERVAL_MS = 30_000;

/**
 * A connection that has had an envelope this long without acknowledging
 * it is asleep: Android froze the app behind it, and the phone's network
 * stack still holds the connection open. It is closed, and the device is
 * woken by push instead.
 */
export const ACK_WAIT_MS = 20_000;

/**
 * At most one call wake-up per mailbox in this long (ADR-0019): a phone
 * that is away has a few seconds to fetch the offer and decide to ring.
 */
export const CALL_WAKE_INTERVAL_MS = 5_000;

/** Calls' server lists one mailbox may ask for per UTC day (ADR-0019). */
export const ICE_QUOTA = 200;

/** Acknowledgement frame tag: `0x01` followed by 16-byte envelope ids. */
const ACK = 0x01;

export type ClaimOutcome = 'claimed' | 'same' | 'taken';
export type DepositOutcome = 'queued' | 'duplicate' | 'unknown' | 'full';
export type UploadAllowance = 'allowed' | 'unauthorized' | 'over_quota';
export type IceAllowance = UploadAllowance;

/**
 * What a deposit asks of a device that is away: a quiet "you have mail",
 * or, for an envelope its sender marked urgent, a wake-up that lets the
 * phone fetch it at once and ring if it is a call (ADR-0019).
 */
export type Wake = 'message' | 'call';

/** Per-connection state that survives hibernation. */
interface Cursor {
  /** Highest `seq` sent on this connection. */
  last: number;
  /**
   * Since when it has had an envelope it did not acknowledge; 0 when it
   * has acknowledged everything sent.
   */
  waiting?: number;
}

export class Mailbox extends DurableObject<Env> {
  /** Cached number of queued envelopes; `null` when unknown. */
  private queued: number | null = null;

  constructor(ctx: DurableObjectState, env: Env) {
    super(ctx, env);
    // The app asks every 20 seconds whether its connection still works
    // (`protocol/delivery.md`); the runtime answers without waking this
    // object.
    ctx.setWebSocketAutoResponse(new WebSocketRequestResponsePair('ping', 'pong'));
  }

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
    if (!(await this.owns(secret))) {
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

  /**
   * Whether the holder of `secret` owns this mailbox and may get one more
   * list of call servers today; counts it if so (ADR-0019). Each list may
   * carry relay credentials, which cost money once used, so they are
   * rationed like uploads.
   */
  async allowIce(secret: ArrayBuffer): Promise<IceAllowance> {
    if (!(await this.owns(secret))) {
      return 'unauthorized';
    }
    const sql = this.ctx.storage.sql;
    sql.exec(
      'CREATE TABLE IF NOT EXISTS ice (id INTEGER PRIMARY KEY CHECK (id = 1), day INTEGER NOT NULL, count INTEGER NOT NULL)',
    );
    const today = Math.floor(Date.now() / 86_400_000);
    const row = sql
      .exec<{ day: number; count: number }>('SELECT day, count FROM ice WHERE id = 1')
      .toArray()[0];
    const used = row?.day === today ? row.count : 0;
    if (used >= ICE_QUOTA) {
      return 'over_quota';
    }
    sql.exec(
      `INSERT INTO ice (id, day, count) VALUES (1, ?, ?)
       ON CONFLICT (id) DO UPDATE SET day = excluded.day, count = excluded.count`,
      today,
      used + 1,
    );
    return 'allowed';
  }

  /**
   * Keeps the push token of this mailbox's device, or forgets it with
   * `null`, for whoever proves the secret (ADR-0017).
   */
  async setPush(secret: ArrayBuffer, token: string | null): Promise<'ok' | 'unauthorized'> {
    if (!(await this.owns(secret))) {
      return 'unauthorized';
    }
    this.pushSchema();
    if (token === null) {
      this.ctx.storage.sql.exec('DELETE FROM push');
    } else {
      this.ctx.storage.sql.exec(
        `INSERT INTO push (id, token, last, pending) VALUES (1, ?, 0, 0)
         ON CONFLICT (id) DO UPDATE SET token = excluded.token`,
        token,
      );
    }
    return 'ok';
  }

  /** Queues one envelope and pushes it to live connections. */
  async deposit(
    id: ArrayBuffer,
    envelope: ArrayBuffer,
    wake: Wake = 'message',
  ): Promise<DepositOutcome> {
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
    const sockets = this.open();
    for (const socket of sockets) {
      this.pump(socket);
    }
    // Owed whether or not a connection is open: if the connection turns
    // out to be asleep, the wake-up goes once it is closed.
    const due = wake === 'call' ? this.callWakeDue() : this.wakeUpDue();
    if (sockets.length === 0 && due) {
      // The wake-up goes out from the alarm, so the sender does not wait
      // for Google.
      await this.ctx.storage.setAlarm(Date.now());
    } else if (sockets.length > 0) {
      // Checks that the connection acknowledges what it was sent.
      await this.alarmBy(Date.now() + ACK_WAIT_MS);
    } else if ((await this.ctx.storage.getAlarm()) === null) {
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
    // It answered: it is awake. Whatever it still has is counted from now.
    const cursor = socket.deserializeAttachment() as Cursor | null;
    if (cursor !== null) {
      socket.serializeAttachment({
        last: cursor.last,
        waiting: this.inFlight(cursor) > 0 ? Date.now() : 0,
      } satisfies Cursor);
    }
    if (this.count() === 0) {
      this.forgetOwed();
    }
    for (const other of this.open()) {
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
    this.closeAsleep();
    await this.sendWakeUp();
    const next = this.ctx.storage.sql
      .exec<{ expires: number }>('SELECT MIN(expires) AS expires FROM envelopes')
      .one().expires;
    const waiting = this.open().some(
      (socket) => ((socket.deserializeAttachment() as Cursor | null)?.waiting ?? 0) > 0,
    );
    if (typeof next === 'number') {
      await this.ctx.storage.setAlarm(waiting ? Math.min(next, Date.now() + ACK_WAIT_MS) : next);
    }
  }

  /** Connections that are open, as far as this object can tell. */
  private open(): WebSocket[] {
    return this.ctx.getWebSockets().filter((socket) => socket.readyState === WebSocket.OPEN);
  }

  /**
   * Closes every connection that has had an envelope for [ACK_WAIT_MS]
   * without acknowledging it. Its envelopes stay queued; the app fetches
   * them when it wakes, and push wakes it.
   */
  private closeAsleep(): void {
    const now = Date.now();
    for (const socket of this.open()) {
      const waiting = (socket.deserializeAttachment() as Cursor | null)?.waiting ?? 0;
      if (waiting > 0 && now - waiting >= ACK_WAIT_MS) {
        try {
          socket.close(1001, 'no acknowledgement');
        } catch {
          // Already closing.
        }
      }
    }
  }

  /** Makes sure the alarm fires by `time`. */
  private async alarmBy(time: number): Promise<void> {
    const current = await this.ctx.storage.getAlarm();
    if (current === null || current > time) {
      await this.ctx.storage.setAlarm(time);
    }
  }

  /** How many envelopes sent on a connection it has not acknowledged. */
  private inFlight(cursor: Cursor): number {
    return this.ctx.storage.sql
      .exec<{ n: number }>('SELECT COUNT(*) AS n FROM envelopes WHERE seq <= ?', cursor.last)
      .one().n;
  }

  /** Sends queued envelopes to `socket` until its window is full. */
  private pump(socket: WebSocket): void {
    const cursor = socket.deserializeAttachment() as Cursor | null;
    if (cursor === null) {
      return;
    }
    const sql = this.ctx.storage.sql;
    const inFlight = this.inFlight(cursor);
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
      // The wait starts with the first envelope it has not acknowledged.
      const waiting = inFlight > 0 && (cursor.waiting ?? 0) > 0 ? cursor.waiting : Date.now();
      socket.serializeAttachment({ last, waiting } satisfies Cursor);
    }
  }

  /** Whether `secret` is this mailbox's. */
  private async owns(secret: ArrayBuffer): Promise<boolean> {
    const owner = this.owner();
    return (
      owner !== null &&
      secret.byteLength === 32 &&
      sameBytes(await sha256(new Uint8Array(secret)), owner)
    );
  }

  private pushSchema(): void {
    this.ctx.storage.sql.exec(`
      CREATE TABLE IF NOT EXISTS push (
        id INTEGER PRIMARY KEY CHECK (id = 1),
        token TEXT NOT NULL,
        last INTEGER NOT NULL,
        pending INTEGER NOT NULL
      )
    `);
  }

  /**
   * Marks a wake-up as owed, if the device has a token and was not woken
   * lately; returns whether one is owed now.
   */
  private wakeUpDue(): boolean {
    this.pushSchema();
    const row = this.ctx.storage.sql
      .exec<{ last: number; pending: number }>('SELECT last, pending FROM push WHERE id = 1')
      .toArray()[0];
    if (row === undefined) {
      return false;
    }
    if (row.pending === 1) {
      return true;
    }
    if (Date.now() - row.last < PUSH_INTERVAL_MS) {
      return false;
    }
    this.ctx.storage.sql.exec('UPDATE push SET pending = 1 WHERE id = 1');
    return true;
  }

  /**
   * Marks a call wake-up as owed, if the device has a token and was not
   * woken for a call in the last few seconds.
   */
  private callWakeDue(): boolean {
    this.pushSchema();
    const sql = this.ctx.storage.sql;
    if (sql.exec('SELECT 1 FROM push WHERE id = 1').toArray().length === 0) {
      return false;
    }
    sql.exec(
      'CREATE TABLE IF NOT EXISTS call_wake (id INTEGER PRIMARY KEY CHECK (id = 1), last INTEGER NOT NULL, pending INTEGER NOT NULL)',
    );
    const row = sql
      .exec<{ last: number; pending: number }>('SELECT last, pending FROM call_wake WHERE id = 1')
      .toArray()[0];
    if (row?.pending === 1) {
      return true;
    }
    if (Date.now() - (row?.last ?? 0) < CALL_WAKE_INTERVAL_MS) {
      return false;
    }
    sql.exec(
      `INSERT INTO call_wake (id, last, pending) VALUES (1, ?, 1)
       ON CONFLICT (id) DO UPDATE SET last = excluded.last, pending = 1`,
      Date.now(),
    );
    return true;
  }

  /**
   * The device read everything: no wake-up is owed any more, even one
   * that waited for its connection to prove asleep.
   */
  private forgetOwed(): void {
    this.pushSchema();
    const sql = this.ctx.storage.sql;
    sql.exec('UPDATE push SET pending = 0 WHERE id = 1');
    if (
      sql.exec("SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'call_wake'").toArray()
        .length > 0
    ) {
      sql.exec('UPDATE call_wake SET pending = 0 WHERE id = 1');
    }
  }

  /**
   * Sends an owed wake-up, unless the device has meanwhile read its mail.
   * While a connection is open, it waits: the connection either answers
   * or is closed as asleep (ACK_WAIT_MS).
   */
  private async sendWakeUp(): Promise<void> {
    if (this.open().length > 0) {
      return;
    }
    this.pushSchema();
    const sql = this.ctx.storage.sql;
    const call =
      sql.exec("SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'call_wake'").toArray()
        .length > 0 &&
      sql.exec('SELECT 1 FROM call_wake WHERE id = 1 AND pending = 1').toArray().length > 0;
    if (call) {
      sql.exec('UPDATE call_wake SET pending = 0 WHERE id = 1');
      const token = sql
        .exec<{ token: string }>('SELECT token FROM push WHERE id = 1')
        .toArray()[0]?.token;
      if (token !== undefined && this.count() > 0 && this.open().length === 0) {
        const outcome = await this.env.PUSH.getByName('fcm').send(token, 'call');
        if (outcome === 'gone') {
          sql.exec('DELETE FROM push WHERE token = ?', token);
          return;
        }
      }
    }
    const row = sql
      .exec<{ token: string; pending: number }>('SELECT token, pending FROM push WHERE id = 1')
      .toArray()[0];
    if (row?.pending !== 1) {
      return;
    }
    sql.exec('UPDATE push SET pending = 0, last = ? WHERE id = 1', Date.now());
    if (this.count() === 0 || this.open().length > 0) {
      return;
    }
    const outcome = await this.env.PUSH.getByName('fcm').send(row.token);
    if (outcome === 'gone') {
      sql.exec('DELETE FROM push WHERE token = ?', row.token);
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
