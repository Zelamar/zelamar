/**
 * Servers that help a call connect (ADR-0019): Cloudflare's STUN server
 * always, and its TURN relay, with short-lived credentials, once the service
 * holds a TURN key. The relay only ever carries media the two devices
 * encrypted for each other.
 */

/** One entry of a WebRTC `iceServers` list. */
export interface IceServer {
  urls: string[];
  username?: string;
  credential?: string;
}

/** Tells a device its public address; free and keyless. */
export const STUN: IceServer = { urls: ['stun:stun.cloudflare.com:3478'] };

/** How long relay credentials work, in seconds: longer than any call. */
export const TURN_TTL = 4 * 3600;

/** The TURN key's id and API token (Cloudflare secrets). */
export interface TurnKey {
  id: string;
  token: string;
}

/** The TURN key from its two secrets; `null` unless both are set. */
export function turnKey(id: string | undefined, token: string | undefined): TurnKey | null {
  return id !== undefined && token !== undefined && id.trim() !== '' && token.trim() !== ''
    ? { id: id.trim(), token: token.trim() }
    : null;
}

/**
 * Port 53 is often blocked; waiting for it would only slow down gathering,
 * since every other port is offered too.
 */
const PORT_53 = /:53(\?|$)/u;

/** The servers for one device's calls: STUN, plus TURN when `key` is set. */
export async function iceServers(
  key: TurnKey | null,
  fetcher: typeof fetch = fetch,
): Promise<IceServer[]> {
  if (key === null) {
    return [STUN];
  }
  try {
    const response = await fetcher(
      `https://rtc.live.cloudflare.com/v1/turn/keys/${encodeURIComponent(key.id)}/credentials/generate-ice-servers`,
      {
        method: 'POST',
        headers: { authorization: `Bearer ${key.token}`, 'content-type': 'application/json' },
        body: JSON.stringify({ ttl: TURN_TTL }),
      },
    );
    if (!response.ok) {
      throw new Error(`TURN credentials: HTTP ${String(response.status)}`);
    }
    const servers = relayServers(await response.json());
    if (servers.length === 0) {
      throw new Error('TURN credentials: no usable servers');
    }
    return servers;
  } catch (error) {
    // A call still connects on most networks without the relay.
    console.error(JSON.stringify({ event: 'turn_failed', message: String(error) }));
    return [STUN];
  }
}

/** The usable entries of Cloudflare's answer, without port 53. */
function relayServers(answer: unknown): IceServer[] {
  const raw = (answer as { iceServers?: unknown } | null)?.iceServers;
  const entries: unknown[] = Array.isArray(raw) ? raw : raw === undefined ? [] : [raw];
  const servers: IceServer[] = [];
  for (const entry of entries) {
    if (typeof entry !== 'object' || entry === null) {
      continue;
    }
    const { urls, username, credential } = entry as Record<string, unknown>;
    const list = (Array.isArray(urls) ? urls : [urls]).filter(
      (url): url is string =>
        typeof url === 'string' && /^(stun|turns?):/u.test(url) && !PORT_53.test(url),
    );
    if (list.length === 0) {
      continue;
    }
    const server: IceServer = { urls: list };
    if (typeof username === 'string' && typeof credential === 'string') {
      server.username = username;
      server.credential = credential;
    }
    servers.push(server);
  }
  return servers;
}
