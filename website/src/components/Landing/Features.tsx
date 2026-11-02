import type { ReactNode } from 'react';
import Link from '@docusaurus/Link';
import Heading from '@theme/Heading';
import styles from './Landing.module.css';

type Feature = {
  title: string;
  text: string;
  to: string;
  color: string;
  icon: ReactNode;
  beta?: boolean;
};

const sv = {
  viewBox: '0 0 24 24',
  width: 24,
  height: 24,
  fill: 'none',
  stroke: 'currentColor',
  strokeWidth: 1.7,
  strokeLinecap: 'round' as const,
  strokeLinejoin: 'round' as const,
  'aria-hidden': true,
};

const FEATURES: Feature[] = [
  {
    title: 'Dashboards',
    text: 'A House overview synthesized from your rooms, plus your own dashboards built from widgets: lights, covers, climate, cameras, charts.',
    to: '/docs/users/web-ui#dashboards',
    color: 'var(--nidoo-amber)',
    icon: (
      <svg {...sv}>
        <rect x="3" y="3" width="8" height="8" rx="2" />
        <rect x="13" y="3" width="8" height="5" rx="2" />
        <rect x="13" y="10" width="8" height="11" rx="2" />
        <rect x="3" y="13" width="8" height="8" rx="2" />
      </svg>
    ),
  },
  {
    title: 'Flows',
    text: 'Automations as small graphs: triggers, checks, waits and actions. Edit them on a canvas and replay what a run did.',
    to: '/docs/users/flows',
    color: 'var(--nidoo-violet)',
    icon: (
      <svg {...sv}>
        <circle cx="5" cy="6" r="2.2" />
        <circle cx="19" cy="6" r="2.2" />
        <circle cx="12" cy="18" r="2.2" />
        <path d="M7 7c4 0 4 2 5 9M17 7c-4 0-4 2-5 9" />
      </svg>
    ),
  },
  {
    title: 'Energy',
    text: 'The live power flow of the house: solar, grid, battery and consumers, with the kWh of each paired counter.',
    to: '/docs/users/web-ui#energy',
    color: 'var(--nidoo-mint)',
    icon: (
      <svg {...sv}>
        <path d="M13 2 4 14h7l-1 8 9-12h-7z" />
      </svg>
    ),
  },
  {
    title: 'Climate',
    text: 'Heating and cooling by zone. The engine learns each room from history and proposes changes you approve or reject.',
    to: '/docs/users/climate',
    color: 'var(--nidoo-cyan)',
    beta: true,
    icon: (
      <svg {...sv}>
        <path d="M14 14.8V5a2 2 0 0 0-4 0v9.8a4 4 0 1 0 4 0z" />
        <path d="M12 9v7" />
      </svg>
    ),
  },
  {
    title: 'Phone alerts and cameras',
    text: 'Push notifications to the Android app through Firebase, with Frigate camera events and their snapshots in the Alerts tab.',
    to: '/docs/users/push-notifications',
    color: 'var(--nidoo-rose)',
    icon: (
      <svg {...sv}>
        <path d="M6 9a6 6 0 0 1 12 0c0 6 2 7 2 7H4s2-1 2-7z" />
        <path d="M10 20a2 2 0 0 0 4 0" />
      </svg>
    ),
  },
  {
    title: 'Talk to your house',
    text: 'The hub is an MCP server. Ask an assistant to inspect devices, build flows or change settings in plain words.',
    to: '/docs/users/assistants-mcp',
    color: 'var(--nidoo-amber)',
    icon: (
      <svg {...sv}>
        <path d="M21 12a8 8 0 0 1-11.6 7.1L4 20l1-4.6A8 8 0 1 1 21 12z" />
        <path d="M8.5 11h.01M12 11h.01M15.5 11h.01" />
      </svg>
    ),
  },
];

export default function Features() {
  return (
    <section className={styles.section}>
      <div className="container">
        <p className={styles.eyebrow}>What it does</p>
        <Heading as="h2" className={styles.title}>
          Everything a house needs, <em>nothing it doesn't.</em>
        </Heading>
        <ul className={styles.grid}>
          {FEATURES.map((f, i) => (
            <li key={f.title} className={styles.card} style={{ animationDelay: `${i * 60}ms` }}>
              <span className={styles.icon} style={{ color: f.color }}>
                {f.icon}
              </span>
              <h3 className={styles.cardTitle}>
                {f.title}
                {f.beta && <span className={styles.beta}>Beta</span>}
              </h3>
              <p className={styles.cardText}>{f.text}</p>
              <Link to={f.to} className={styles.cardLink}>
                Read more <span aria-hidden="true">→</span>
              </Link>
            </li>
          ))}
        </ul>
      </div>
    </section>
  );
}
