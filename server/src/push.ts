/**
 * Wake-ups through Firebase Cloud Messaging (ADR-0017): one object talks to
 * Google, keeps the access token for its lifetime, and sends a fixed,
 * content-free notification to a device token. Without the service
 * account secret it only counts what it would have sent.
 */
import { DurableObject } from 'cloudflare:workers';

import { toBase64Url, utf8 } from './bytes';

/** The parts of a Firebase service account key that sending needs. */
export interface ServiceAccount {
  project_id: string;
  client_email: string;
  private_key: string;
  token_uri: string;
}

export type PushOutcome =
  /** Google accepted it. */
  | 'sent'
  /** No service account configured: counted, not sent. */
  | 'unconfigured'
  /** The token is no longer valid: forget it. */
  | 'gone'
  /** Anything else; the next message tries again. */
  | 'failed';

const SCOPE = 'https://www.googleapis.com/auth/firebase.messaging';

/** Reads the `FCM_SERVICE_ACCOUNT` secret; `null` when absent or unusable. */
export function serviceAccount(secret: string | undefined): ServiceAccount | null {
  if (secret === undefined || secret.trim() === '') {
    return null;
  }
  try {
    const value: unknown = JSON.parse(secret);
    if (typeof value !== 'object' || value === null) {
      return null;
    }
    const fields = value as Record<string, unknown>;
    const { project_id, client_email, private_key } = fields;
    const token_uri = fields.token_uri ?? 'https://oauth2.googleapis.com/token';
    return typeof project_id === 'string' &&
      typeof client_email === 'string' &&
      typeof private_key === 'string' &&
      typeof token_uri === 'string'
      ? { project_id, client_email, private_key, token_uri }
      : null;
  } catch {
    return null;
  }
}

/** The DER bytes inside a PEM block. */
function pemBody(pem: string): Uint8Array {
  const base64 = pem.replace(/-----(BEGIN|END) [A-Z ]+-----/gu, '').replace(/\s+/gu, '');
  return Uint8Array.from(atob(base64), (char) => char.charCodeAt(0));
}

/**
 * The signed JWT that Google exchanges for an access token (RFC 7523),
 * issued at `now` seconds.
 */
export async function assertion(account: ServiceAccount, now: number): Promise<string> {
  const header = toBase64Url(utf8(JSON.stringify({ alg: 'RS256', typ: 'JWT' })));
  const claims = toBase64Url(
    utf8(
      JSON.stringify({
        iss: account.client_email,
        scope: SCOPE,
        aud: account.token_uri,
        iat: now,
        exp: now + 3600,
      }),
    ),
  );
  const key = await crypto.subtle.importKey(
    'pkcs8',
    pemBody(account.private_key),
    { name: 'RSASSA-PKCS1-v1_5', hash: 'SHA-256' },
    false,
    ['sign'],
  );
  const signature = await crypto.subtle.sign('RSASSA-PKCS1-v1_5', key, utf8(`${header}.${claims}`));
  return `${header}.${claims}.${toBase64Url(new Uint8Array(signature))}`;
}

/** The FCM message for `token`: the same words for every wake-up. */
export function wakeUp(token: string): Record<string, unknown> {
  return {
    message: {
      token,
      notification: { title: 'Zelamar', body: 'New message' },
      android: {
        priority: 'HIGH',
        ttl: '3600s',
        collapse_key: 'zelamar',
        notification: { channel_id: 'messages', tag: 'zelamar' },
      },
    },
  };
}

export class Push extends DurableObject<Env> {
  private access: { token: string; until: number } | null = null;

  /** Sends one wake-up to the device holding `token`. */
  async send(token: string): Promise<PushOutcome> {
    const account = serviceAccount(this.env.FCM_SERVICE_ACCOUNT);
    if (account === null) {
      this.count();
      return 'unconfigured';
    }
    try {
      const response = await fetch(
        `https://fcm.googleapis.com/v1/projects/${account.project_id}/messages:send`,
        {
          method: 'POST',
          headers: {
            authorization: `Bearer ${await this.accessToken(account)}`,
            'content-type': 'application/json',
          },
          body: JSON.stringify(wakeUp(token)),
        },
      );
      if (response.ok) {
        console.log(JSON.stringify({ event: 'push_sent' }));
        return 'sent';
      }
      const text = await response.text();
      if (response.status === 404 || text.includes('UNREGISTERED')) {
        return 'gone';
      }
      if (response.status === 401) {
        this.access = null;
      }
      console.error(JSON.stringify({ event: 'push_failed', status: response.status }));
      return 'failed';
    } catch (error) {
      console.error(JSON.stringify({ event: 'push_failed', message: String(error) }));
      return 'failed';
    }
  }

  /** How many wake-ups were skipped for want of a service account. */
  skipped(): number {
    this.schema();
    return this.ctx.storage.sql.exec<{ n: number }>('SELECT n FROM skipped WHERE id = 1').one().n;
  }

  private count(): void {
    this.schema();
    this.ctx.storage.sql.exec('UPDATE skipped SET n = n + 1 WHERE id = 1');
  }

  private schema(): void {
    this.ctx.storage.sql.exec(`
      CREATE TABLE IF NOT EXISTS skipped (id INTEGER PRIMARY KEY CHECK (id = 1), n INTEGER NOT NULL);
      INSERT INTO skipped (id, n) VALUES (1, 0) ON CONFLICT (id) DO NOTHING;
    `);
  }

  private async accessToken(account: ServiceAccount): Promise<string> {
    const cached = this.access;
    if (cached !== null && cached.until > Date.now()) {
      return cached.token;
    }
    const jwt = await assertion(account, Math.floor(Date.now() / 1000));
    const response = await fetch(account.token_uri, {
      method: 'POST',
      headers: { 'content-type': 'application/x-www-form-urlencoded' },
      body: new URLSearchParams({
        grant_type: 'urn:ietf:params:oauth:grant-type:jwt-bearer',
        assertion: jwt,
      }),
    });
    const answer: { access_token?: unknown; expires_in?: unknown } = await response.json();
    if (!response.ok || typeof answer.access_token !== 'string') {
      throw new Error(`token exchange failed: ${String(response.status)}`);
    }
    const lifetime = typeof answer.expires_in === 'number' ? answer.expires_in : 3600;
    this.access = { token: answer.access_token, until: Date.now() + (lifetime - 60) * 1000 };
    return answer.access_token;
  }
}
