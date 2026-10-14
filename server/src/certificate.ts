/**
 * Device certificates (`protocol/identity.md`): an account key vouches for
 * one device's signature key.
 */
import { concat, utf8 } from './bytes';
import { verifyEd25519 } from './crypto';

const CONTEXT = utf8('zelamar/device-certificate/v0');
const VERSION = 0;

/** A decoded device certificate. */
export interface DeviceCertificate {
  account: Uint8Array;
  device: Uint8Array;
  deviceKey: Uint8Array;
  signature: Uint8Array;
}

/** Decodes a version 0 certificate; `null` if malformed. */
export function decodeCertificate(bytes: Uint8Array): DeviceCertificate | null {
  let offset = 0;
  const take = (length: number): Uint8Array | null => {
    if (offset + length > bytes.length) {
      return null;
    }
    const slice = bytes.subarray(offset, offset + length);
    offset += length;
    return slice;
  };
  const prefixed = (): Uint8Array | null => {
    const length = take(2);
    return length === null ? null : take(((length[0] ?? 0) << 8) | (length[1] ?? 0));
  };

  const version = take(1);
  const account = take(32);
  const device = take(16);
  const deviceKey = prefixed();
  const signature = prefixed();
  if (
    version?.[0] !== VERSION ||
    account === null ||
    device === null ||
    deviceKey?.length !== 32 ||
    signature?.length !== 64 ||
    offset !== bytes.length
  ) {
    return null;
  }
  return { account, device, deviceKey, signature };
}

/** Whether the certificate's signature verifies under its account key. */
export async function verifyCertificate(certificate: DeviceCertificate): Promise<boolean> {
  const keyLength = new Uint8Array([0, certificate.deviceKey.length]);
  const payload = concat(
    CONTEXT,
    certificate.account,
    certificate.device,
    keyLength,
    certificate.deviceKey,
  );
  return verifyEd25519(certificate.account, payload, certificate.signature);
}
