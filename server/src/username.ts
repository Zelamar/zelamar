/**
 * `@username` rules (`protocol/directory.md`).
 */

/** Names nobody may claim: the service's own, and roles people trust. */
const RESERVED = new Set([
  'abuse',
  'admin',
  'administrator',
  'all',
  'api',
  'billing',
  'bot',
  'ceo',
  'contact',
  'cto',
  'everyone',
  'founder',
  'help',
  'helpdesk',
  'here',
  'info',
  'legal',
  'mail',
  'me',
  'mod',
  'moderator',
  'news',
  'null',
  'official',
  'payments',
  'privacy',
  'root',
  'security',
  'service',
  'settings',
  'staff',
  'status',
  'support',
  'system',
  'team',
  'undefined',
  'updates',
  'verified',
  'www',
]);

/** Lowercased `input` if it is a well-formed username, else `null`. */
export function normalizeUsername(input: unknown): string | null {
  // ASCII only, checked before lowercasing: Unicode lowercasing turns some
  // letters (the Kelvin sign) into ASCII ones.
  if (typeof input !== 'string' || !/^[A-Za-z0-9_]{3,32}$/u.test(input)) {
    return null;
  }
  const name = input.toLowerCase();
  const wellFormed = /^[a-z][a-z0-9_]*$/u.test(name) && !name.includes('__') && !name.endsWith('_');
  return wellFormed ? name : null;
}

/** Whether a well-formed `name` is reserved (including anything "zelamar"). */
export function isReserved(name: string): boolean {
  return RESERVED.has(name) || name.includes('zelamar');
}
