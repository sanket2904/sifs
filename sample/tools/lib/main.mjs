import { realpathSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

// Whether the module at `url` is the script node was started with. Node gives
// a module its real path, so the script's path is resolved the same way; a
// script started through a symlinked folder (/tmp on macOS) still runs.
export const isMain = (url) =>
	Boolean(process.argv[1]) && realpathSync(process.argv[1]) === fileURLToPath(url);
