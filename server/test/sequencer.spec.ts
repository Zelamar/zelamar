import { exports } from 'cloudflare:workers';
import { describe, expect, it } from 'vitest';

import { utf8 } from '../src/bytes';
import { type CommitFrame, decodeCommit, encodeCommit } from '../src/frames';
import { BASE, b64, envelopeId, inbox, random, registered } from './helpers';

function post(group: Uint8Array, frame: CommitFrame): Promise<Response> {
  return exports.default.fetch(`${BASE}/v0/groups/${b64(group)}/commits`, {
    method: 'POST',
    body: encodeCommit(frame),
  });
}

describe('the commit sequencer', () => {
  it('lets the first commit of an epoch win, refuses rivals, and repeats retries', async () => {
    const bob = await registered();
    const carol = await registered();
    const group = random(16);

    // A commit for Bob that welcomes Carol, made in epoch 3.
    const first: CommitFrame = {
      epoch: 3,
      commit: { id: random(16), recipients: [bob.device.mailbox], payload: utf8('commit A') },
      welcome: { id: random(16), recipients: [carol.device.mailbox], payload: utf8('welcome') },
    };
    const accepted = await post(group, first);
    expect(accepted.status).toBe(202);
    expect(await accepted.json()).toEqual({ epoch: 4, delivered: 1, welcomed: 1 });

    // A rival commit for the same epoch loses and learns the current one.
    const rival: CommitFrame = {
      epoch: 3,
      commit: { id: random(16), recipients: [bob.device.mailbox], payload: utf8('commit B') },
      welcome: null,
    };
    const refused = await post(group, rival);
    expect(refused.status).toBe(409);
    expect(await refused.json()).toEqual({ error: 'epoch_mismatch', epoch: 4 });

    // A retry of the winner is accepted again and delivered only once.
    const retried = await post(group, first);
    expect(retried.status).toBe(202);
    expect(await retried.json()).toEqual({ epoch: 4, delivered: 1, welcomed: 1 });

    // The next epoch moves on.
    const next = await post(group, {
      epoch: 4,
      commit: { id: random(16), recipients: [], payload: utf8('commit C') },
      welcome: null,
    });
    expect(await next.json()).toEqual({ epoch: 5, delivered: 0, welcomed: 0 });

    const bobs = await inbox(bob.device);
    expect(envelopeId(await bobs.next())).toEqual(first.commit.id);
    expect(await bobs.settle()).toBe(0);
    bobs.close();
    const carols = await inbox(carol.device);
    expect(envelopeId(await carols.next())).toEqual(first.welcome?.id);
    carols.close();
  });

  it('refuses malformed frames and unknown group ids', async () => {
    const group = random(16);
    const frame = encodeCommit({
      epoch: 1,
      commit: { id: random(16), recipients: [random(32)], payload: utf8('x') },
      welcome: null,
    });
    const send = (path: string, body: Uint8Array) =>
      exports.default.fetch(`${BASE}/v0/groups/${path}/commits`, { method: 'POST', body });

    expect((await send(b64(group), frame.subarray(0, 12))).status).toBe(400);
    const wrongMagic = frame.slice();
    wrongMagic[1] = 0;
    expect((await send(b64(group), wrongMagic)).status).toBe(400);
    expect((await send('short', frame)).status).toBe(404);
    expect((await exports.default.fetch(`${BASE}/v0/groups/${b64(group)}/commits`)).status).toBe(
      405,
    );
  });

  it('round-trips commit frames, with and without a welcome', () => {
    const withWelcome: CommitFrame = {
      epoch: 7,
      commit: { id: random(16), recipients: [random(32), random(32)], payload: utf8('c') },
      welcome: { id: random(16), recipients: [random(32)], payload: utf8('w') },
    };
    expect(decodeCommit(encodeCommit(withWelcome))).toEqual(withWelcome);
    const alone: CommitFrame = { ...withWelcome, welcome: null };
    expect(decodeCommit(encodeCommit(alone))).toEqual(alone);
  });
});
