import type { BaseLayoutProps } from 'fumadocs-ui/layouts/shared';

export const baseOptions: BaseLayoutProps = {
  nav: {
    title: (
      <>
        <span className="font-mono font-semibold">🐙 tako</span>
      </>
    ),
  },
  links: [
    {
      text: 'Documentation',
      url: '/docs',
      active: 'nested-url',
    },
    {
      text: 'crates.io',
      url: 'https://crates.io/crates/tako-rs',
      external: true,
    },
    {
      text: 'docs.rs',
      url: 'https://docs.rs/tako-rs',
      external: true,
    },
    {
      text: 'GitHub',
      url: 'https://github.com/rust-dd/tako',
      external: true,
    },
  ],
  githubUrl: 'https://github.com/rust-dd/tako',
};
