/**
 * Binary frames: the delivery frame a sender posts (`protocol/delivery.md`)
 * and the envelope a mailbox stores (`protocol/envelope.md`).
 */
import { concat, toBase64Url, u32 } from './bytes';

/** Largest payload accepted (1 MiB), as in the core. */
export const MAX_PAYLOAD = 1 << 20;
/** Most recipients one delivery frame may name. */
export const MAX_RECIPIENTS = 255;
/** Largest possible delivery frame. */
export const MAX_DELIVERY_FRAME = 2 + 1 + 16 + 1 + 32 * MAX_RECIPIENTS + 4 + MAX_PAYLOAD;

const DELIVERY_MAGIC = [0x5a, 0x44]; // "ZD"
const ENVELOPE_MAGIC = [0x5a, 0x45]; // "ZE"
const VERSION = 0;

/** A decoded delivery frame. */
export interface Delivery {
  id: Uint8Array;
  recipients: Uint8Array[];
  payload: Uint8Array;
}

/** Decodes a delivery frame; a string names what is wrong with it. */
export function decodeDelivery(bytes: Uint8Array): Delivery | 'malformed' | 'too_large' {
  const read = readDelivery(bytes, false);
  if (typeof read === 'string') {
    return read;
  }
  return read.length === bytes.length ? read.delivery : 'malformed';
}

/**
 * Reads one delivery frame from the start of [bytes]; returns it with the
 * number of bytes it takes. [allowEmpty] admits a frame with no recipients
 * (a commit nobody else needs, which still advances the epoch).
 */
function readDelivery(
  bytes: Uint8Array,
  allowEmpty: boolean,
): { delivery: Delivery; length: number } | 'malformed' | 'too_large' {
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const header = 2 + 1 + 16 + 1;
  if (
    bytes.length < header ||
    bytes[0] !== DELIVERY_MAGIC[0] ||
    bytes[1] !== DELIVERY_MAGIC[1] ||
    bytes[2] !== VERSION
  ) {
    return 'malformed';
  }
  const id = bytes.subarray(3, 19);
  const count = bytes[19] ?? 0;
  const lengthAt = header + 32 * count;
  if ((count === 0 && !allowEmpty) || bytes.length < lengthAt + 4) {
    return 'malformed';
  }
  const recipients: Uint8Array[] = [];
  const seen = new Set<string>();
  for (let i = 0; i < count; i++) {
    const recipient = bytes.subarray(header + 32 * i, header + 32 * (i + 1));
    const key = toBase64Url(recipient);
    if (seen.has(key)) {
      return 'malformed';
    }
    seen.add(key);
    recipients.push(recipient);
  }
  const length = view.getUint32(lengthAt);
  if (length > MAX_PAYLOAD) {
    return 'too_large';
  }
  const end = lengthAt + 4 + length;
  if (bytes.length < end) {
    return 'malformed';
  }
  return { delivery: { id, recipients, payload: bytes.subarray(lengthAt + 4, end) }, length: end };
}

const COMMIT_MAGIC = [0x5a, 0x43]; // "ZC"

/** A commit for the chat's sequencer (`protocol/delivery.md`). */
export interface CommitFrame {
  /** The epoch the commit was made in. */
  epoch: number;
  /** The commit, for the members before it. */
  commit: Delivery;
  /** The welcome, for members it adds. */
  welcome: Delivery | null;
}

/** Largest possible commit frame: two delivery frames and a header. */
export const MAX_COMMIT_FRAME = 2 + 1 + 8 + 2 * MAX_DELIVERY_FRAME;

/** Decodes a commit frame; a string names what is wrong with it. */
export function decodeCommit(bytes: Uint8Array): CommitFrame | 'malformed' | 'too_large' {
  const header = 2 + 1 + 8;
  if (
    bytes.length < header ||
    bytes[0] !== COMMIT_MAGIC[0] ||
    bytes[1] !== COMMIT_MAGIC[1] ||
    bytes[2] !== VERSION
  ) {
    return 'malformed';
  }
  const epoch = new DataView(bytes.buffer, bytes.byteOffset + 3, 8).getBigUint64(0);
  if (epoch > BigInt(Number.MAX_SAFE_INTEGER)) {
    return 'malformed';
  }
  const commit = readDelivery(bytes.subarray(header), true);
  if (typeof commit === 'string') {
    return commit;
  }
  const rest = bytes.subarray(header + commit.length);
  if (rest.length === 0) {
    return { epoch: Number(epoch), commit: commit.delivery, welcome: null };
  }
  const welcome = readDelivery(rest, false);
  if (typeof welcome === 'string') {
    return welcome;
  }
  if (welcome.length !== rest.length) {
    return 'malformed';
  }
  return { epoch: Number(epoch), commit: commit.delivery, welcome: welcome.delivery };
}

/** Encodes a commit frame (used by tests and future tooling). */
export function encodeCommit(frame: CommitFrame): Uint8Array {
  const epoch = new Uint8Array(8);
  new DataView(epoch.buffer).setBigUint64(0, BigInt(frame.epoch));
  return concat(
    Uint8Array.from([...COMMIT_MAGIC, VERSION]),
    epoch,
    encodeDelivery(frame.commit),
    ...(frame.welcome === null ? [] : [encodeDelivery(frame.welcome)]),
  );
}

/** Encodes a delivery frame (used by tests and future tooling). */
export function encodeDelivery(delivery: Delivery): Uint8Array {
  return concat(
    Uint8Array.from([...DELIVERY_MAGIC, VERSION]),
    delivery.id,
    Uint8Array.of(delivery.recipients.length),
    ...delivery.recipients,
    u32(delivery.payload.length),
    delivery.payload,
  );
}

/** The envelope stored in, and handed out by, one mailbox. */
export function encodeEnvelope(
  id: Uint8Array,
  mailbox: Uint8Array,
  payload: Uint8Array,
): Uint8Array {
  return concat(
    Uint8Array.from([...ENVELOPE_MAGIC, VERSION]),
    id,
    mailbox,
    u32(payload.length),
    payload,
  );
}
