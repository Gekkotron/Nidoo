import { themes as prismThemes } from 'prism-react-renderer';
import type { Config } from '@docusaurus/types';
import type * as Preset from '@docusaurus/preset-classic';

const config: Config = {
  title: 'Nidoo',
  tagline: 'A hub you run at home, and talk to.',
  favicon: 'img/favicon.svg',

  url: 'https://gekkotron.github.io',
  baseUrl: '/nidoo/',

  organizationName: 'Gekkotron',
  projectName: 'nidoo',
  trailingSlash: false,

  onBrokenLinks: 'throw',

  markdown: {
    // The pages are CommonMark `.md` full of `<id>` placeholders: `detect`
    // parses `.md` as CommonMark and only `.mdx` as MDX.
    format: 'detect',
    hooks: {
      onBrokenMarkdownLinks: 'throw',
    },
  },

  headTags: [
    {
      tagName: 'link',
      attributes: { rel: 'apple-touch-icon', href: '/nidoo/img/apple-touch-icon.png' },
    },
  ],

  presets: [
    [
      'classic',
      {
        docs: {
          path: '../docs',
          routeBasePath: 'docs',
          sidebarPath: './sidebars.ts',
          editUrl: ({ docPath }) =>
            `https://github.com/Gekkotron/nidoo/edit/main/docs/${docPath}`,
          exclude: ['superpowers/**', '**/_*.{js,jsx,ts,tsx,md,mdx}', '**/_*/**'],
        },
        blog: false,
        theme: {
          customCss: './src/css/custom.css',
        },
      } satisfies Preset.Options,
    ],
  ],

  themeConfig: {
    image: 'img/screens/house.webp',
    colorMode: {
      defaultMode: 'dark',
      respectPrefersColorScheme: true,
    },
    navbar: {
      title: 'Nidoo',
      logo: { alt: 'Nidoo logo', src: 'img/logo.svg' },
      items: [
        { to: '/docs/category/user-guide', label: 'User guide', position: 'left' },
        { to: '/docs/category/reference', label: 'Reference', position: 'left' },
        { to: '/docs/category/contributors', label: 'Contributors', position: 'left' },
        {
          href: 'https://github.com/Gekkotron/nidoo',
          label: 'GitHub',
          position: 'right',
        },
      ],
    },
    footer: {
      style: 'light',
      links: [
        {
          title: 'Docs',
          items: [
            { label: 'Getting started', to: '/docs/users/getting-started' },
            { label: 'Install with Docker', to: '/docs/users/install-on-linux' },
            { label: 'Talking to the house', to: '/docs/users/assistants-mcp' },
            { label: 'Configuration', to: '/docs/reference/configuration' },
          ],
        },
        {
          title: 'Project',
          items: [
            { label: 'GitHub', href: 'https://github.com/Gekkotron/nidoo' },
            { label: 'Issues', href: 'https://github.com/Gekkotron/nidoo/issues' },
            {
              label: 'License',
              href: 'https://github.com/Gekkotron/nidoo/blob/main/LICENSE',
            },
          ],
        },
      ],
      copyright: 'Nidoo · Built by Gekkotron · Apache-2.0',
    },
    prism: {
      theme: prismThemes.github,
      darkTheme: prismThemes.vsDark,
      additionalLanguages: ['toml', 'bash', 'json', 'rust', 'yaml'],
    },
  } satisfies Preset.ThemeConfig,
};

export default config;
