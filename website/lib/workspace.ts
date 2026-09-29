import { readFileSync } from 'node:fs';
import { join } from 'node:path';

export const workspaceRoot = join(process.cwd(), '..');

export function readWorkspaceFile(path: string) {
  return readFileSync(join(workspaceRoot, path), 'utf8');
}

export const crateVersion =
  /^\[workspace\.package\][^[]*?^version = "([^"]+)"/m.exec(readWorkspaceFile('Cargo.toml'))?.[1] ??
  '2.0.0';

export const crateMinorVersion = crateVersion.split('.').slice(0, 2).join('.');
