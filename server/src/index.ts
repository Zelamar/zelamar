/**
 * Zelamar delivery service: delivers everything, knows nothing (ADR-0012).
 *
 * The Worker routes and runs stateless checks; state lives in Durable
 * Objects: `Account` per @username, `Mailbox` per device mailbox, `Group`
 * per chat (the commit sequencer), `Link` per device-linking slot, and
 * `Invites` for the closed beta. The service never sees plaintext or who
 * sent a message.
 */
import packageJson from '../package.json';
import { buffer, fixedBase64Url, toBase64Url } from './bytes';
import { checkClaim } from './claim';
import {
  decodeCommit,
  decodeDelivery,
  type Delivery,
  encodeEnvelope,
  MAX_COMMIT_FRAME,
  MAX_DELIVERY_FRAME,
} from './frames';
import { failure, json, parseObject, readBody, respond, type Reply } from './http';
import { MAX_CHUNK, MAX_CHUNKS } from './blob';
import { inviteKey, inviteSerial } from './invite';
import { type LinkPart, linkMessage, MAX_LINK_ANSWER } from './link';
import type { SignedRequest } from './signed';
import { normalizeUsername } from './username';

export { Account, Invites } from './account';
export { Blob } from './blob';
export { Group } from './group';
export { Link } from './link';
export { Mailbox } from './mailbox';
export { Push } from './push';

const SERVICE_NAME = 'zelamar';
/** Largest JSON body: 100 key packages of 16 KiB, base64url, with room. */
const MAX_JSON_BODY = 3 * 1024 * 1024;
/** Largest claim body. */
const MAX_CLAIM_BODY = 4 * 1024;
/** Mailboxes written to at once while fanning out one delivery. */
const FAN_OUT_BATCH = 8;

export default {
  async fetch(request, env): Promise<Response> {
    try {
      return await route(request, env);
    } catch (error) {
      // Never leak internals; the trace is in Workers Logs.
      console.error(JSON.stringify({ event: 'unhandled', message: String(error) }));
      return json({ error: 'internal' }, 500);
    }
  },
} satisfies ExportedHandler<Env>;

async function route(request: Request, env: Env): Promise<Response> {
  const { pathname } = new URL(request.url);
  const parts = pathname.split('/').filter((part) => part !== '');

  if (pathname === '/health') {
    return health(request);
  }
  if (parts[0] !== 'v0') {
    return json({ error: 'not_found' }, 404);
  }
  const [, resource, name, sub, device, leaf] = parts;

  if (resource === 'deliver' && parts.length === 2) {
    return only(request, 'POST') ?? respond(await deliver(request, env));
  }
  if (resource === 'groups' && name !== undefined && sub === 'commits' && parts.length === 4) {
    return only(request, 'POST') ?? respond(await commit(request, env, name));
  }
  if (resource === 'mailboxes' && name !== undefined && sub === 'push' && parts.length === 4) {
    if (request.method !== 'PUT' && request.method !== 'DELETE') {
      return json({ error: 'method_not_allowed' }, 405, { allow: 'PUT, DELETE' });
    }
    return respond(await pushToken(request, env, name));
  }
  if (resource === 'mailboxes' && name !== undefined && parts.length === 3) {
    return only(request, 'GET') ?? mailbox(request, env, name);
  }
  if (resource === 'accounts' && parts.length === 2) {
    return only(request, 'POST') ?? respond(await claim(request, env));
  }
  if (resource === 'accounts' && name !== undefined && sub === undefined) {
    return only(request, 'GET') ?? respond(await lookup(request, env, name, 'devices'));
  }
  if (resource === 'accounts' && name !== undefined && sub === 'key' && parts.length === 4) {
    return only(request, 'GET') ?? respond(await lookup(request, env, name, 'key'));
  }
  if (resource === 'accounts' && name !== undefined && sub === 'devices' && parts.length === 4) {
    return only(request, 'GET') ?? respond(await lookup(request, env, name, 'list'));
  }
  if (resource === 'accounts' && name !== undefined && sub === 'devices' && device !== undefined) {
    if (parts.length === 5 && request.method === 'GET') {
      return respond(await lookup(request, env, name, { device }));
    }
    if (parts.length === 5 && request.method === 'DELETE') {
      return respond(await signed(request, env, name, device, 'remove'));
    }
    if (parts.length === 5) {
      return only(request, 'PUT') ?? respond(await signed(request, env, name, device, 'register'));
    }
    if (parts.length === 6 && leaf === 'key-packages') {
      return only(request, 'POST') ?? respond(await signed(request, env, name, device, 'publish'));
    }
  }
  if (resource === 'blobs' && name !== undefined && sub !== undefined && parts.length === 4) {
    if (request.method === 'PUT') {
      return respond(await putChunk(request, env, name, sub));
    }
    return only(request, 'GET') ?? getChunk(request, env, name, sub);
  }
  if (resource === 'links' && name !== undefined && parts.length === 3) {
    return linkSlot(request, env, name);
  }
  if (resource === 'links' && name !== undefined && parts.length === 4) {
    if (sub === 'request' || sub === 'answer') {
      return only(request, 'PUT') ?? respond(await putLink(request, env, name, sub));
    }
  }
  return json({ error: 'not_found' }, 404);
}

function health(request: Request): Response {
  if (request.method !== 'GET' && request.method !== 'HEAD') {
    return json({ error: 'method_not_allowed' }, 405, { allow: 'GET, HEAD' });
  }
  return json({ status: 'ok', service: SERVICE_NAME, version: packageJson.version });
}

/** `null` if `request` uses `method`, else a 405 response. */
function only(request: Request, method: string): Response | null {
  return request.method === method
    ? null
    : json({ error: 'method_not_allowed' }, 405, { allow: method });
}

/** Whether the caller's network address is still within `limiter`'s budget. */
async function allowed(limiter: RateLimit, request: Request): Promise<boolean> {
  const key = request.headers.get('cf-connecting-ip') ?? 'unknown';
  return (await limiter.limit({ key })).success;
}

async function deliver(request: Request, env: Env): Promise<Reply> {
  if (!(await allowed(env.DELIVER_LIMITER, request))) {
    return failure(429, 'rate_limited');
  }
  const body = await readBody(request, MAX_DELIVERY_FRAME);
  if (body === null) {
    return failure(413, 'too_large');
  }
  const frame = decodeDelivery(body);
  if (frame === 'malformed') {
    return failure(400, 'malformed');
  }
  if (frame === 'too_large') {
    return failure(413, 'too_large');
  }

  return { status: 202, body: { delivered: await fanOut(env, frame) } };
}

/** Puts one envelope per recipient into their mailboxes; returns how many took it. */
async function fanOut(env: Env, delivery: Delivery): Promise<number> {
  let delivered = 0;
  for (let at = 0; at < delivery.recipients.length; at += FAN_OUT_BATCH) {
    const batch = delivery.recipients.slice(at, at + FAN_OUT_BATCH);
    const outcomes = await Promise.all(
      batch.map((recipient) =>
        env.MAILBOX.getByName(toBase64Url(recipient)).deposit(
          buffer(delivery.id),
          buffer(encodeEnvelope(delivery.id, recipient, delivery.payload)),
        ),
      ),
    );
    delivered += outcomes.filter(
      (outcome) => outcome === 'queued' || outcome === 'duplicate',
    ).length;
  }
  return delivered;
}

/** A commit through its chat's sequencer: the first one for an epoch wins. */
async function commit(request: Request, env: Env, group: string): Promise<Reply> {
  if (fixedBase64Url(group, 16) === null) {
    return failure(404, 'unknown_group');
  }
  if (!(await allowed(env.DELIVER_LIMITER, request))) {
    return failure(429, 'rate_limited');
  }
  const body = await readBody(request, MAX_COMMIT_FRAME);
  if (body === null) {
    return failure(413, 'too_large');
  }
  const frame = decodeCommit(body);
  if (frame === 'malformed') {
    return failure(400, 'malformed');
  }
  if (frame === 'too_large') {
    return failure(413, 'too_large');
  }
  const advance = await env.GROUP.getByName(group).advance(frame.epoch, buffer(frame.commit.id));
  if (advance.outcome === 'mismatch') {
    return { status: 409, body: { error: 'epoch_mismatch', epoch: advance.epoch } };
  }
  // The commit first, then the welcome: newcomers can only use a welcome
  // once the others are in the new epoch.
  const delivered = await fanOut(env, frame.commit);
  const welcomed = frame.welcome === null ? 0 : await fanOut(env, frame.welcome);
  return { status: 202, body: { epoch: advance.epoch, delivered, welcomed } };
}

/** Largest push registration body. */
const MAX_PUSH_BODY = 8 * 1024;

/** Keeps (PUT) or forgets (DELETE) a mailbox's push token (ADR-0017). */
async function pushToken(request: Request, env: Env, name: string): Promise<Reply> {
  const secret = /^Bearer ([A-Za-z0-9_-]+)$/u.exec(request.headers.get('authorization') ?? '');
  const secretBytes = fixedBase64Url(secret?.[1], 32);
  if (fixedBase64Url(name, 32) === null || secretBytes === null) {
    return failure(401, 'unauthorized');
  }
  if (!(await allowed(env.DELIVER_LIMITER, request))) {
    return failure(429, 'rate_limited');
  }
  let token: string | null = null;
  if (request.method === 'PUT') {
    const body = await readBody(request, MAX_PUSH_BODY);
    const fields = body === null ? null : parseObject(body);
    const candidate = fields?.token;
    if (
      fields?.platform !== 'fcm' ||
      typeof candidate !== 'string' ||
      candidate.length === 0 ||
      candidate.length > 4096
    ) {
      return failure(400, 'malformed');
    }
    token = candidate;
  }
  const outcome = await env.MAILBOX.getByName(name).setPush(buffer(secretBytes), token);
  return outcome === 'ok' ? { status: 200, body: {} } : failure(401, 'unauthorized');
}

function mailbox(request: Request, env: Env, name: string): Promise<Response> | Response {
  if (fixedBase64Url(name, 32) === null) {
    return json({ error: 'unknown_mailbox' }, 404);
  }
  return env.MAILBOX.getByName(name).fetch(request);
}

async function claim(request: Request, env: Env): Promise<Reply> {
  if (!(await allowed(env.CLAIM_LIMITER, request))) {
    return failure(429, 'rate_limited');
  }
  const body = await readBody(request, MAX_CLAIM_BODY);
  const fields = body === null ? null : parseObject(body);
  if (fields === null) {
    return failure(400, 'malformed');
  }
  const checked = await checkClaim(fields, Number(env.WORK_BITS), Date.now());
  if ('status' in checked) {
    return checked;
  }
  const { username, account } = checked;

  const holder = env.ACCOUNT.getByName(username);
  const state = await holder.state(buffer(account));
  if (state === 'same') {
    return { status: 200, body: { username } };
  }
  if (state === 'taken') {
    return failure(409, 'username_taken');
  }
  const key = inviteKey(env.INVITE_KEY);
  const serial = key === null ? null : await inviteSerial(key, checked.invite);
  if (serial === null || !(await env.INVITES.getByName('beta').redeem(serial))) {
    return failure(403, 'invalid_invite');
  }
  const outcome = await holder.claim(username, buffer(account));
  return outcome === 'taken'
    ? failure(409, 'username_taken')
    : { status: outcome === 'created' ? 201 : 200, body: { username } };
}

/**
 * A directory lookup: one key package per device (`devices`), only the
 * account key (`key`), only the devices' certificates (`list`), or one key
 * package of one device.
 */
async function lookup(
  request: Request,
  env: Env,
  name: string,
  what: 'devices' | 'key' | 'list' | { device: string },
): Promise<Reply> {
  const username = normalizeUsername(name);
  if (username === null) {
    return failure(400, 'invalid_username');
  }
  if (!(await allowed(env.LOOKUP_LIMITER, request))) {
    return failure(429, 'rate_limited');
  }
  const holder = env.ACCOUNT.getByName(username);
  if (what === 'devices') {
    return holder.lookup();
  }
  if (what === 'key') {
    return holder.keyOf();
  }
  if (what === 'list') {
    return holder.listDevices();
  }
  const device = fixedBase64Url(what.device, 16);
  return device === null ? failure(404, 'unknown_device') : holder.lookupDevice(buffer(device));
}

/** A chunk index from the path, or `null`. */
function chunkIndex(text: string): number | null {
  return /^\d{1,2}$/u.test(text) && Number(text) < MAX_CHUNKS ? Number(text) : null;
}

/** Stores one sealed file chunk for a device that proves its mailbox (ADR-0016). */
async function putChunk(request: Request, env: Env, blob: string, at: string): Promise<Reply> {
  const index = chunkIndex(at);
  if (fixedBase64Url(blob, 16) === null || index === null) {
    return failure(404, 'unknown_blob');
  }
  const count = Number(request.headers.get('zelamar-chunks') ?? '');
  if (!Number.isInteger(count) || count < 1 || count > MAX_CHUNKS || index >= count) {
    return failure(400, 'malformed');
  }
  const mailbox = request.headers.get('zelamar-mailbox') ?? '';
  const secret = /^Bearer ([A-Za-z0-9_-]+)$/u.exec(request.headers.get('authorization') ?? '');
  const secretBytes = fixedBase64Url(secret?.[1], 32);
  if (fixedBase64Url(mailbox, 32) === null || secretBytes === null) {
    return failure(401, 'unauthorized');
  }
  if (!(await allowed(env.UPLOAD_LIMITER, request))) {
    return failure(429, 'rate_limited');
  }
  const body = await readBody(request, MAX_CHUNK);
  if (body === null) {
    return failure(413, 'too_large');
  }
  if (body.length < 16) {
    return failure(400, 'malformed');
  }
  const allowance = await env.MAILBOX.getByName(mailbox).allowUpload(
    buffer(secretBytes),
    body.length,
  );
  if (allowance !== 'allowed') {
    return allowance === 'over_quota' ? failure(429, 'over_quota') : failure(401, 'unauthorized');
  }
  const outcome = await env.BLOB.getByName(blob).put(index, count, buffer(body));
  if (outcome === 'taken') {
    return failure(409, 'blob_taken');
  }
  return { status: outcome === 'stored' ? 201 : 200, body: {} };
}

/** Hands out one sealed file chunk to whoever knows the blob id. */
async function getChunk(request: Request, env: Env, blob: string, at: string): Promise<Response> {
  const index = chunkIndex(at);
  if (fixedBase64Url(blob, 16) === null || index === null) {
    return json({ error: 'unknown_blob' }, 404);
  }
  if (!(await allowed(env.DOWNLOAD_LIMITER, request))) {
    return json({ error: 'rate_limited' }, 429);
  }
  const chunk = await env.BLOB.getByName(blob).get(index);
  if (chunk === null) {
    return json({ error: 'unknown_blob' }, 404);
  }
  return new Response(chunk, {
    headers: {
      'content-type': 'application/octet-stream',
      'cache-control': 'no-store',
      'x-content-type-options': 'nosniff',
    },
  });
}

/** Reads (GET) or clears (DELETE) a link slot (ADR-0014). */
async function linkSlot(request: Request, env: Env, slot: string): Promise<Response> {
  if (request.method !== 'GET' && request.method !== 'DELETE') {
    return json({ error: 'method_not_allowed' }, 405, { allow: 'GET, DELETE' });
  }
  if (fixedBase64Url(slot, 16) === null) {
    return json({ error: 'unknown_link' }, 404);
  }
  if (!(await allowed(env.LINK_LIMITER, request))) {
    return json({ error: 'rate_limited' }, 429);
  }
  const link = env.LINK.getByName(slot);
  if (request.method === 'DELETE') {
    await link.clear();
    return new Response(null, { status: 204 });
  }
  const found = await link.read();
  if (found === null) {
    return json({ error: 'unknown_link' }, 404);
  }
  return json({
    request: toBase64Url(new Uint8Array(found.request)),
    answer: found.answer === null ? null : toBase64Url(new Uint8Array(found.answer)),
  });
}

/** Leaves a sealed request or answer in a link slot, if it is the first. */
async function putLink(request: Request, env: Env, slot: string, part: LinkPart): Promise<Reply> {
  if (fixedBase64Url(slot, 16) === null) {
    return failure(404, 'unknown_link');
  }
  if (!(await allowed(env.LINK_LIMITER, request))) {
    return failure(429, 'rate_limited');
  }
  const body = await readBody(request, MAX_LINK_ANSWER);
  if (body === null) {
    return failure(413, 'too_large');
  }
  if (!linkMessage(part, body)) {
    return failure(400, 'malformed');
  }
  switch (await env.LINK.getByName(slot).put(part, buffer(body))) {
    case 'stored':
      return { status: 201, body: {} };
    case 'same':
      return { status: 200, body: {} };
    case 'taken':
      return failure(409, 'link_taken');
    case 'unknown':
      return failure(404, 'unknown_link');
  }
}

async function signed(
  request: Request,
  env: Env,
  name: string,
  device: string,
  action: 'register' | 'publish' | 'remove',
): Promise<Reply> {
  const username = normalizeUsername(name);
  if (username === null || name !== username) {
    return failure(404, 'unknown_account');
  }
  const body = await readBody(request, MAX_JSON_BODY);
  if (body === null) {
    return failure(413, 'too_large');
  }
  const parts: SignedRequest = {
    method: request.method,
    path: new URL(request.url).pathname,
    certificate: request.headers.get('zelamar-certificate'),
    timestamp: request.headers.get('zelamar-timestamp'),
    signature: request.headers.get('zelamar-signature'),
    body: buffer(body),
  };
  const holder = env.ACCOUNT.getByName(username);
  if (action === 'remove') {
    return holder.removeDevice(device, parts, request.headers.get('zelamar-account-signature'));
  }
  return action === 'register'
    ? holder.registerDevice(device, parts)
    : holder.publishKeyPackages(device, parts);
}
