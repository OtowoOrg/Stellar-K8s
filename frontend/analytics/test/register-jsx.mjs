// Registered via `node --import ./test/register-jsx.mjs` so the `.jsx` loader
// hook is active before any test module is evaluated.

import { register } from 'node:module';

register('./jsx-hooks.mjs', import.meta.url);
