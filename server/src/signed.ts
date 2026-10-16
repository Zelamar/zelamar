/**
 * Signed requests (`protocol/directory.md`): a certified device, or the
 * account key itself, proves it made exactly this request, recently.
 */
import { fixedBase64Url, fromBase64Url } from './bytes';
import { decodeCertificate, verifyCertificate } from './certificate';
import { fresh, requestPayload, sameBytes, verifyEd25519 } from './crypto';
import { failure, type Reply } from './http';

/** The parts of a signed device request needed to check it. */
export interface SignedRequest {
  method: string;
  path: string;
  certificate: string | null;
  timestamp: string | null;
  signature: string | null;
  body: ArrayBuffer;
}

/** A request that checked out: the device and its certificate. */
export interface SignedBy {
  device: Uint8Array;
  certificate: Uint8Array;
}

/**
 * Checks that `request` was signed at about `now` by the device `device`
 * (base64url, from the path), certified by the account key `accountKey`.
 */
export async function checkSignedRequest(
  request: SignedRequest,
  accountKey: Uint8Array,
  device: string,
  now: number,
): Promise<SignedBy | Reply> {
  const deviceId = fixedBase64Url(device, 16);
  const certificateBytes = fromBase64Url(request.certificate ?? '');
  const certificate = certificateBytes === null ? null : decodeCertificate(certificateBytes);
  const signature = fixedBase64Url(request.signature, 64);
  if (
    deviceId === null ||
    certificateBytes === null ||
    certificate === null ||
    signature === null ||
    !sameBytes(certificate.account, accountKey) ||
    !sameBytes(certificate.device, deviceId) ||
    !(await verifyCertificate(certificate))
  ) {
    return failure(401, 'unauthorized');
  }
  const timestamp = timestampOf(request);
  if (!fresh(timestamp, now)) {
    return failure(400, 'stale_request');
  }
  const payload = await requestPayload(
    request.method,
    request.path,
    timestamp,
    new Uint8Array(request.body),
  );
  if (!(await verifyEd25519(certificate.deviceKey, payload, signature))) {
    return failure(401, 'unauthorized');
  }
  return { device: deviceId, certificate: certificateBytes };
}

/**
 * Checks that `request` was signed at about `now` by the account key
 * `accountKey` itself; `request.signature` is the `Zelamar-Account-Signature`
 * header.
 */
export async function checkAccountRequest(
  request: SignedRequest,
  accountKey: Uint8Array,
  now: number,
): Promise<true | Reply> {
  const signature = fixedBase64Url(request.signature, 64);
  if (signature === null) {
    return failure(401, 'unauthorized');
  }
  const timestamp = timestampOf(request);
  if (!fresh(timestamp, now)) {
    return failure(400, 'stale_request');
  }
  const payload = await requestPayload(
    request.method,
    request.path,
    timestamp,
    new Uint8Array(request.body),
    'zelamar/account-request/v0',
  );
  return (await verifyEd25519(accountKey, payload, signature))
    ? true
    : failure(401, 'unauthorized');
}

function timestampOf(request: SignedRequest): number {
  return /^\d{1,16}$/u.test(request.timestamp ?? '') ? Number(request.timestamp) : NaN;
}
