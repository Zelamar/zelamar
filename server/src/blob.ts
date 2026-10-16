/**
 * One stored file (ADR-0016, `protocol/attachments.md`): its sealed chunks,
 * opaque to the service, each written once. The object's name is the blob
 * id, which only the people in the chat know. Everything is gone 30 days
 * after the first chunk arrives.
 */
import { DurableObject } from 'cloudflare:workers';

import { sameBytes } from './crypto';

/** Plaintext bytes per chunk, plus the 16-byte tag of a sealed chunk. */
export const MAX_CHUNK = (1 << 20) + 16;
/** Most chunks in one blob (25 MiB of plaintext). */
export const MAX_CHUNKS = 25;
/** How long a blob is kept. */
export const BLOB_TTL_MS = 30 * 24 * 60 * 60 * 1000;

export type PutChunk =
  /** Kept. */
  | 'stored'
  /** The same chunk again: a harmless retry. */
  | 'same'
  /** Different bytes for a chunk already stored, or another chunk count. */
  | 'taken';

export class Blob extends DurableObject<Env> {
  /** Keeps chunk `index` of a blob of `count` chunks, if it is the first. */
  async put(index: number, count: number, chunk: ArrayBuffer): Promise<PutChunk> {
    const sql = this.ctx.storage.sql;
    sql.exec(`
      CREATE TABLE IF NOT EXISTS blob (
        id INTEGER PRIMARY KEY CHECK (id = 1),
        count INTEGER NOT NULL,
        expires INTEGER NOT NULL
      );
      CREATE TABLE IF NOT EXISTS chunks (idx INTEGER PRIMARY KEY, data BLOB NOT NULL);
    `);
    const meta = sql.exec<{ count: number }>('SELECT count FROM blob WHERE id = 1').toArray()[0];
    if (meta === undefined) {
      const expires = Date.now() + BLOB_TTL_MS;
      sql.exec('INSERT INTO blob (id, count, expires) VALUES (1, ?, ?)', count, expires);
      await this.ctx.storage.setAlarm(expires);
    } else if (meta.count !== count) {
      return 'taken';
    }
    const stored = sql
      .exec<{ data: ArrayBuffer }>('SELECT data FROM chunks WHERE idx = ?', index)
      .toArray()[0];
    if (stored !== undefined) {
      return sameBytes(new Uint8Array(stored.data), new Uint8Array(chunk)) ? 'same' : 'taken';
    }
    sql.exec('INSERT INTO chunks (idx, data) VALUES (?, ?)', index, chunk);
    return 'stored';
  }

  /** Chunk `index`, or `null` if there is no such chunk (yet, or any more). */
  get(index: number): ArrayBuffer | null {
    const sql = this.ctx.storage.sql;
    const exists = sql
      .exec("SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'chunks'")
      .toArray().length;
    if (exists === 0) {
      return null;
    }
    const meta = sql
      .exec<{ expires: number }>('SELECT expires FROM blob WHERE id = 1')
      .toArray()[0];
    // The alarm may run late; an expired blob is gone all the same.
    if (meta === undefined || meta.expires <= Date.now()) {
      return null;
    }
    return (
      sql.exec<{ data: ArrayBuffer }>('SELECT data FROM chunks WHERE idx = ?', index).toArray()[0]
        ?.data ?? null
    );
  }

  override async alarm(): Promise<void> {
    await this.ctx.storage.deleteAll();
  }
}
