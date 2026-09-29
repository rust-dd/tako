import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { ServerCodeBlock } from 'fumadocs-ui/components/codeblock.rsc';

export interface RustExampleProps {
  /**
   * Path to the example file, relative to the workspace root
   * (one directory above this `website/` folder).
   * Example: `examples/auth/src/main.rs` or `tako-rs-core/tests/routing.rs`.
   */
  path: string;
}

export function RustExample({ path }: RustExampleProps) {
  const workspaceRoot = join(process.cwd(), '..');
  const filePath = join(workspaceRoot, path);

  let source: string;
  try {
    source = readFileSync(filePath, 'utf8');
  } catch {
    return (
      <pre className="rounded-md border border-red-500/40 bg-red-500/5 p-4 text-sm text-red-600">
        {`<RustExample path="${path}" /> — file not found at ${filePath}`}
      </pre>
    );
  }

  return (
    <ServerCodeBlock
      code={source.trimEnd()}
      lang="rust"
      themes={{ light: 'github-light', dark: 'github-dark' }}
      codeblock={{ title: path }}
    />
  );
}
