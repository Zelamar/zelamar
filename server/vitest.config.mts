import { cloudflareTest } from '@cloudflare/vitest-plugin';
import { defineConfig } from 'vitest/config';

export default defineConfig({
  plugins: [
    cloudflareTest({
      wrangler: { configPath: './wrangler.jsonc' },
      miniflare: {
        bindings: {
          // Test-only values: a public invite key and an easy proof of work.
          INVITE_KEY: '7a656c616d61722d746573742d696e766974652d6b65792d6e6f742d7365637265',
          WORK_BITS: '4',
          // No Firebase in tests: wake-ups are counted, not sent.
          FCM_SERVICE_ACCOUNT: '',
        },
      },
    }),
  ],
});
