import clsx from 'clsx';
import Link from '@docusaurus/Link';
import useBaseUrl from '@docusaurus/useBaseUrl';
import Layout from '@theme/Layout';
import Heading from '@theme/Heading';
import ThemedImage from '@theme/ThemedImage';
import Features from '../components/Landing/Features';
import Showcase from '../components/Landing/Showcase';
import TalkToIt from '../components/Landing/TalkToIt';
import styles from './index.module.css';

function Hero() {
  return (
    <header className={styles.hero}>
      <div className={styles.sky} aria-hidden="true" />
      <div className={clsx('container', styles.heroInner)}>
        <div className={styles.markWrap}>
          <ThemedImage
            sources={{
              dark: useBaseUrl('/img/mark-animated.svg'),
              light: useBaseUrl('/img/mark-animated-ivoire.svg'),
            }}
            width={88}
            height={88}
            alt=""
          />
        </div>
        <p className={styles.preamble}>
          <span>open source</span>
          <span className={styles.dot} aria-hidden="true">·</span>
          <span>self-hosted</span>
          <span className={styles.dot} aria-hidden="true">·</span>
          <span>LAN only</span>
        </p>
        <Heading as="h1" className={styles.headline}>
          <span className={styles.line1}>Your house, one hub.</span>
          <span className={styles.line2}>Ask it anything.</span>
        </Heading>
        <p className={styles.subhead}>
          Nidoo runs your Zigbee2MQTT devices, dashboards, flows, energy, climate and phone
          alerts from one small Rust hub. You set it up by talking to it over MCP.
        </p>
        <div className={styles.ctas}>
          <Link to="/docs/users/getting-started" className={clsx('button button--lg', styles.ctaPrimary)}>
            Get started <span aria-hidden="true">→</span>
          </Link>
          <Link to="/docs/users/install-on-linux" className={clsx('button button--lg', styles.ctaGhost)}>
            Install with Docker
          </Link>
          <Link href="https://github.com/Gekkotron/nidoo" className={styles.ctaText}>
            View the source <span aria-hidden="true">↗</span>
          </Link>
        </div>
        <ul className={styles.chips} aria-label="Project facts">
          {['Rust', 'Zigbee2MQTT', 'MCP-native', 'Apache-2.0'].map((c) => (
            <li key={c} className={styles.chip}>
              {c}
            </li>
          ))}
        </ul>
      </div>
    </header>
  );
}

function ProductShot() {
  return (
    <section className={styles.shotSection} aria-label="The web UI">
      <div className="container">
        <div className={styles.frame}>
          <div className={styles.chrome} aria-hidden="true">
            <span className={styles.light} />
            <span className={styles.light} />
            <span className={styles.light} />
            <span className={styles.url}>http://hub.local:8081 · House</span>
          </div>
          <ThemedImage
            sources={{
              dark: useBaseUrl('/img/screens/house.webp'),
              light: useBaseUrl('/img/screens/house-ivoire.webp'),
            }}
            className={styles.shot}
            width={2880}
            height={2270}
            alt="The Nidoo House dashboard: rooms in the sidebar, the weather and solar cards, moods and favourite devices."
          />
        </div>
      </div>
    </section>
  );
}

function Closing() {
  return (
    <section className={styles.closing}>
      <div className="container">
        <Heading as="h2" className={styles.closingTitle}>
          Bring the house home.
        </Heading>
        <p className={styles.closingText}>
          One <code>docker compose up</code> on any Linux box next to your broker.
        </p>
        <div className={styles.ctas}>
          <Link to="/docs/users/getting-started" className={clsx('button button--lg', styles.ctaPrimary)}>
            Get started <span aria-hidden="true">→</span>
          </Link>
          <Link to="/docs/users/install-on-linux" className={clsx('button button--lg', styles.ctaGhost)}>
            Install with Docker
          </Link>
        </div>
        <p className={styles.note}>
          Version 1 has no authentication. Keep the hub on your LAN and off the internet. See{' '}
          <Link to="/docs/users/security">Security</Link>.
        </p>
      </div>
    </section>
  );
}

export default function Home() {
  return (
    <Layout
      title="Your house, one hub"
      description="Nidoo is a self-hosted home-automation hub for Zigbee2MQTT, written in Rust, that you configure by talking to it over MCP.">
      <Hero />
      <main>
        <ProductShot />
        <Features />
        <Showcase />
        <TalkToIt />
        <Closing />
      </main>
    </Layout>
  );
}
