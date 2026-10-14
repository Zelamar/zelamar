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
  if (count === 0 || bytes.length < lengthAt + 4) {
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
  if (bytes.length !== lengthAt + 4 + length) {
    return 'malformed';
  }
  return { id, recipients, payload: bytes.subarray(lengthAt + 4) };
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
