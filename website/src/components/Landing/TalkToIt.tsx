import Link from '@docusaurus/Link';
import Heading from '@theme/Heading';
import styles from './Landing.module.css';

export default function TalkToIt() {
  return (
    <section className={styles.section}>
      <div className="container">
        <div className={styles.talk}>
          <div className={styles.talkText}>
            <p className={styles.eyebrow}>MCP-native</p>
            <Heading as="h2" className={styles.rowTitle}>
              Talk to it, instead of a rules editor.
            </Heading>
            <p className={styles.rowBody}>
              Add the hub to Claude Code, Codex or any MCP client with one line. The assistant sees the whole house and can create flows,
              rename rooms and read history for you.
            </p>
            <Link to="/docs/users/assistants-mcp" className={styles.cardLink}>
              Connect an assistant <span aria-hidden="true">→</span>
            </Link>
          </div>
          <div className={styles.term} role="img" aria-label="Terminal: add the MCP server, then ask for a flow in plain language.">
            <div className={styles.termBar} aria-hidden="true">
              <span className={styles.light} />
              <span className={styles.light} />
              <span className={styles.light} />
            </div>
            <pre className={styles.termBody} aria-hidden="true">
              <span className={styles.prompt}>$ </span>
              <span>claude mcp add --transport http nidoo http://&lt;host&gt;:8081/mcp</span>
              {'\n\n'}
              <span className={styles.you}>you  </span>
              <span>Turn the living room lights on at sunset, and off at midnight.</span>
              {'\n\n'}
              <span className={styles.bot}>nidoo</span>
              <span> Done. Two flows created and enabled.</span>
            </pre>
          </div>
        </div>
      </div>
    </section>
  );
}
