/**
 * One chat's sequencer (ADR-0012): MLS breaks if two members commit in the
 * same epoch and each applies their own, so the first commit for an epoch
 * wins and every other is refused. The object keeps the current epoch and
 * the winning commit's id; nothing about who is in the chat.
 */
import { DurableObject } from 'cloudflare:workers';

import { sameBytes } from './crypto';

export type Advance =
  /** The commit won: deliver it. `epoch` is the new current epoch. */
  | { outcome: 'accepted'; epoch: number }
  /** The same commit again (a retry): deliver it again, harmlessly. */
  | { outcome: 'retry'; epoch: number }
  /** Another commit won this epoch; `epoch` is the current one. */
  | { outcome: 'mismatch'; epoch: number };

export class Group extends DurableObject<Env> {
  /** Lets a commit made in `epoch` advance the chat, if it is the first. */
  advance(epoch: number, commit: ArrayBuffer): Advance {
    const sql = this.ctx.storage.sql;
    sql.exec(`
      CREATE TABLE IF NOT EXISTS state (
        id INTEGER PRIMARY KEY CHECK (id = 1),
        epoch INTEGER NOT NULL,
        last BLOB NOT NULL
      )
    `);
    const row = sql
      .exec<{ epoch: number; last: ArrayBuffer }>('SELECT epoch, last FROM state WHERE id = 1')
      .toArray()[0];
    // A chat seen for the first time starts wherever its members are:
    // chats begin with a commit nobody else receives.
    if (row === undefined || row.epoch === epoch) {
      sql.exec(
        `INSERT INTO state (id, epoch, last) VALUES (1, ?, ?)
         ON CONFLICT (id) DO UPDATE SET epoch = excluded.epoch, last = excluded.last`,
        epoch + 1,
        commit,
      );
      return { outcome: 'accepted', epoch: epoch + 1 };
    }
    if (row.epoch === epoch + 1 && sameBytes(new Uint8Array(row.last), new Uint8Array(commit))) {
      return { outcome: 'retry', epoch: row.epoch };
    }
    return { outcome: 'mismatch', epoch: row.epoch };
  }
}
