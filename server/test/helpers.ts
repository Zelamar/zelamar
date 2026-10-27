/**
 * Test helpers: real Ed25519 keys, certificates, proofs of work and signed
 * requests, built exactly as `protocol/` specifies, so the tests exercise
 * the same checks a real client meets.
 */
import { env, exports } from 'cloudflare:workers';

import { concat, toBase64Url, u64, utf8 } from '../src/bytes';
import { leadingZeroBits, requestPayload, sha256 } from '../src/crypto';
import { encodeDelivery } from '../src/frames';
import { inviteKey, makeInvite } from '../src/invite';

export const BASE = 'https://api.zelamar.test';

export const b64 = toBase64Url;

export function random(length: number): Uint8Array {
  return crypto.getRandomValues(new Uint8Array(length));
}

/** A fresh network address, so per-address rate limits never couple tests. */
export function address(): string {
  return `203.0.113.${String(Math.floor(Math.random() * 250) + 1)}-${b64(random(6))}`;
}

export interface Keys {
  publicKey: Uint8Array;
  privateKey: CryptoKey;
}

export async function keys(): Promise<Keys> {
  const pair = (await crypto.subtle.generateKey({ name: 'Ed25519' }, true, [
    'sign',
    'verify',
  ])) as CryptoKeyPair;
  const raw = (await crypto.subtle.exportKey('raw', pair.publicKey)) as ArrayBuffer;
  return { publicKey: new Uint8Array(raw), privateKey: pair.privateKey };
}

export async function sign(key: Keys, message: Uint8Array): Promise<Uint8Array> {
  return new Uint8Array(await crypto.subtle.sign({ name: 'Ed25519' }, key.privateKey, message));
}

export async function invite(): Promise<string> {
  const key = inviteKey(env.INVITE_KEY);
  if (key === null) {
    throw new Error('test INVITE_KEY is not configured');
  }
  return makeInvite(key, crypto.getRandomValues(new Uint32Array(1))[0] ?? 0);
}

function nameField(username: string): Uint8Array {
  const name = utf8(username);
  return concat(Uint8Array.of(name.length), name);
}

/** Proof of work for `username` that does (or, with `valid: false`, does not) pass. */
export async function work(account: Keys, username: string, valid = true): Promise<Uint8Array> {
  for (;;) {
    const candidate = random(8);
    const proof = await sha256(
      concat(utf8('zelamar/work/v0'), account.publicKey, nameField(username), candidate),
    );
    if (leadingZeroBits(proof) >= Number(env.WORK_BITS) === valid) {
      return candidate;
    }
  }
}

/** A complete, valid claim; `overrides` replace fields after signing. */
export async function claimBody(
  account: Keys,
  username: string,
  overrides: Record<string, unknown> = {},
  timestamp = Date.now(),
): Promise<Record<string, unknown>> {
  const proof = await work(account, username);
  const signature = await sign(
    account,
    concat(utf8('zelamar/claim/v0'), nameField(username), u64(timestamp), proof),
  );
  return {
    username,
    account: b64(account.publicKey),
    invite: await invite(),
    work: b64(proof),
    timestamp,
    signature: b64(signature),
    ...overrides,
  };
}

export function post(path: string, body: unknown): Promise<Response> {
  return exports.default.fetch(BASE + path, {
    method: 'POST',
    body: JSON.stringify(body),
    headers: { 'cf-connecting-ip': address() },
  });
}

export function get(path: string, headers: Record<string, string> = {}): Promise<Response> {
  return exports.default.fetch(BASE + path, {
    headers: { 'cf-connecting-ip': address(), ...headers },
  });
}

export interface Device {
  account: Keys;
  key: Keys;
  id: Uint8Array;
  certificate: Uint8Array;
  mailbox: Uint8Array;
  secret: Uint8Array;
}

/** A new device of `account`, certified as `protocol/identity.md` specifies. */
export async function device(account: Keys): Promise<Device> {
  const key = await keys();
  const id = random(16);
  const keyField = concat(Uint8Array.of(0, 32), key.publicKey);
  const signature = await sign(
    account,
    concat(utf8('zelamar/device-certificate/v0'), account.publicKey, id, keyField),
  );
  const certificate = concat(
    Uint8Array.of(0),
    account.publicKey,
    id,
    keyField,
    Uint8Array.of(0, 64),
    signature,
  );
  return { account, key, id, certificate, mailbox: random(32), secret: random(32) };
}

/** A request signed by `signer` (by default with its own certificate). */
export async function signedFetch(
  signer: Device,
  method: string,
  path: string,
  body: unknown,
  options: { timestamp?: number; certificate?: Uint8Array } = {},
): Promise<Response> {
  const timestamp = options.timestamp ?? Date.now();
  const bytes = utf8(JSON.stringify(body));
  const signature = await sign(signer.key, await requestPayload(method, path, timestamp, bytes));
  return exports.default.fetch(BASE + path, {
    method,
    body: bytes,
    headers: {
      'zelamar-certificate': b64(options.certificate ?? signer.certificate),
      'zelamar-timestamp': String(timestamp),
      'zelamar-signature': b64(signature),
    },
  });
}

export function devicePath(username: string, id: Uint8Array): string {
  return `/v0/accounts/${username}/devices/${b64(id)}`;
}

export async function registrationBody(
  of: Device,
  keyPackages: Uint8Array[] = [],
): Promise<Record<string, unknown>> {
  return {
    mailbox: b64(of.mailbox),
    mailbox_secret_hash: b64(await sha256(of.secret)),
    key_packages: keyPackages.map(b64),
    last_resort: b64(utf8(`last-resort-${b64(of.id)}`)),
  };
}

let names = 0;

/** A unique valid username for this test run. */
export function username(prefix = 'user'): string {
  names += 1;
  return `${prefix}${String(names)}x${b64(random(3))
    .toLowerCase()
    .replace(/[^a-z0-9]/gu, '')}`;
}

/** Claims a fresh username and registers one device with `keyPackages`. */
export async function registered(
  keyPackages: Uint8Array[] = [],
): Promise<{ name: string; account: Keys; device: Device }> {
  const name = username();
  const account = await keys();
  const claimed = await post('/v0/accounts', await claimBody(account, name));
  if (claimed.status !== 201) {
    throw new Error(`claim failed: ${String(claimed.status)} ${await claimed.text()}`);
  }
  const phone = await device(account);
  const response = await signedFetch(
    phone,
    'PUT',
    devicePath(name, phone.id),
    await registrationBody(phone, keyPackages),
  );
  if (response.status !== 200) {
    throw new Error(`registration failed: ${String(response.status)} ${await response.text()}`);
  }
  return { name, account, device: phone };
}

export function deliver(
  id: Uint8Array,
  recipients: Uint8Array[],
  payload: Uint8Array,
  headers: Record<string, string> = {},
): Promise<Response> {
  return exports.default.fetch(`${BASE}/v0/deliver`, {
    method: 'POST',
    body: encodeDelivery({ id, recipients, payload }),
    headers: { 'cf-connecting-ip': address(), ...headers },
  });
}

/** A live connection to a mailbox, with received envelopes in arrival order. */
export class Inbox {
  private readonly received: Uint8Array[] = [];
  private readonly waiting: ((envelope: Uint8Array) => void)[] = [];

  constructor(readonly socket: WebSocket) {
    socket.accept();
    socket.addEventListener('message', (event) => {
      // workerd hands binary messages to the client side as Blobs; reading
      // them is asynchronous, so a chain keeps arrival order.
      const data: unknown = event.data;
      this.arrivals = this.arrivals.then(async () => {
        if (data instanceof Blob) {
          this.take(new Uint8Array(await data.arrayBuffer()));
        } else if (data instanceof ArrayBuffer) {
          this.take(new Uint8Array(data));
        }
      });
    });
  }

  private arrivals = Promise.resolve();

  private take(envelope: Uint8Array): void {
    const waiter = this.waiting.shift();
    if (waiter === undefined) {
      this.received.push(envelope);
    } else {
      waiter(envelope);
    }
  }

  next(): Promise<Uint8Array> {
    const ready = this.received.shift();
    if (ready !== undefined) {
      return Promise.resolve(ready);
    }
    return new Promise((resolve) => this.waiting.push(resolve));
  }

  /** Envelopes that arrived and were not taken yet, after a short pause. */
  async settle(): Promise<number> {
    await new Promise((resolve) => setTimeout(resolve, 100));
    return this.received.length;
  }

  ack(...ids: Uint8Array[]): void {
    this.socket.send(concat(Uint8Array.of(1), ...ids));
  }

  close(): void {
    this.socket.close(1000, 'done');
  }
}

export async function connect(mailbox: Uint8Array, secret: Uint8Array): Promise<Inbox | Response> {
  const response = await exports.default.fetch(`${BASE}/v0/mailboxes/${b64(mailbox)}`, {
    headers: { upgrade: 'websocket', authorization: `Bearer ${b64(secret)}` },
  });
  return response.webSocket === null ? response : new Inbox(response.webSocket);
}

export async function inbox(of: Device): Promise<Inbox> {
  const connection = await connect(of.mailbox, of.secret);
  if (!(connection instanceof Inbox)) {
    throw new Error(`could not connect: ${String(connection.status)}`);
  }
  return connection;
}

/** The envelope id of an envelope (`protocol/envelope.md`). */
export function envelopeId(envelope: Uint8Array): Uint8Array {
  return envelope.subarray(3, 19);
}

/** The payload of an envelope. */
export function envelopePayload(envelope: Uint8Array): Uint8Array {
  return envelope.subarray(55);
}
