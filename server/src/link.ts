/**
 * One rendezvous slot for linking a device (ADR-0014, `protocol/linking.md`):
 * the new device's sealed request, then the sealed answer, each first come
 * first served. The slot is named after the link code, which the service
 * never sees, so it cannot open either message. Everything is gone 10
 * minutes after the request arrives, or as soon as the new device is done.
 */
import { DurableObject } from 'cloudflare:workers';

import { sameBytes } from './crypto';

/** How long a slot lives after its request arrives. */
export const LINK_LIFETIME_MS = 10 * 60 * 1000;
/** Largest sealed request. */
export const MAX_LINK_REQUEST = 2 * 1024;
/** Largest sealed answer: a certificate, a username and the contacts. */
export const MAX_LINK_ANSWER = 64 * 1024;

/** Every sealed link message starts with `ZL`, version 0, then its kind. */
const REQUEST_HEADER = [0x5a, 0x4c, 0x00, 0x01];
const ANSWER_HEADER = [0x5a, 0x4c, 0x00, 0x02];

export type LinkPart = 'request' | 'answer';

export type Put =
  /** Kept. */
  | 'stored'
  /** The same message again: a harmless retry. */
  | 'same'
  /** Another message got there first. */
  | 'taken'
  /** An answer with no request to answer. */
  | 'unknown';

export interface Slot {
  request: ArrayBuffer;
  answer: ArrayBuffer | null;
}

/** Whether `sealed` looks like a link message of kind `part`. */
export function linkMessage(part: LinkPart, sealed: Uint8Array): boolean {
  const header = part === 'request' ? REQUEST_HEADER : ANSWER_HEADER;
  const limit = part === 'request' ? MAX_LINK_REQUEST : MAX_LINK_ANSWER;
  return (
    sealed.length > header.length &&
    sealed.length <= limit &&
    header.every((byte, at) => sealed[at] === byte)
  );
}

export class Link extends DurableObject<Env> {
  /** Keeps `sealed` as the slot's request or answer, if it is the first. */
  async put(part: LinkPart, sealed: ArrayBuffer): Promise<Put> {
    const sql = this.ctx.storage.sql;
    const slot = this.slot();
    if (part === 'request') {
      if (slot !== null) {
        return sameBytes(new Uint8Array(slot.request), new Uint8Array(sealed)) ? 'same' : 'taken';
      }
      const now = Date.now();
      sql.exec(`
        CREATE TABLE IF NOT EXISTS slot (
          id INTEGER PRIMARY KEY CHECK (id = 1),
          request BLOB NOT NULL,
          answer BLOB,
          expires INTEGER NOT NULL
        )
      `);
      // A slot whose alarm has not run yet may still hold an expired row.
      sql.exec(
        `INSERT INTO slot (id, request, answer, expires) VALUES (1, ?, NULL, ?)
         ON CONFLICT (id) DO UPDATE SET
           request = excluded.request, answer = NULL, expires = excluded.expires`,
        sealed,
        now + LINK_LIFETIME_MS,
      );
      await this.ctx.storage.setAlarm(now + LINK_LIFETIME_MS);
      return 'stored';
    }
    if (slot === null) {
      return 'unknown';
    }
    if (slot.answer !== null) {
      return sameBytes(new Uint8Array(slot.answer), new Uint8Array(sealed)) ? 'same' : 'taken';
    }
    sql.exec('UPDATE slot SET answer = ? WHERE id = 1', sealed);
    return 'stored';
  }

  /** What the slot holds; `null` if nothing, or once it expired. */
  read(): Slot | null {
    return this.slot();
  }

  /** Forgets everything; the new device calls this once it is linked. */
  async clear(): Promise<void> {
    await this.ctx.storage.deleteAlarm();
    await this.ctx.storage.deleteAll();
  }

  override async alarm(): Promise<void> {
    await this.ctx.storage.deleteAll();
  }

  private slot(): Slot | null {
    const sql = this.ctx.storage.sql;
    const exists = sql
      .exec("SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'slot'")
      .toArray().length;
    if (exists === 0) {
      return null;
    }
    const row = sql
      .exec<{ request: ArrayBuffer; answer: ArrayBuffer | null; expires: number }>(
        'SELECT request, answer, expires FROM slot WHERE id = 1',
      )
      .toArray()[0];
    // The alarm may run late; an expired slot is gone all the same.
    if (row === undefined || row.expires <= Date.now()) {
      return null;
    }
    return { request: row.request, answer: row.answer };
  }
}
