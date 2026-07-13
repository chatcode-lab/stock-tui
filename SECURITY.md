# Security Policy

## Supported Versions

`stock-tui` is pre-1.0. Security fixes are applied to the latest published
minor release and the default branch. Older binaries may not receive patches.

| Version | Supported |
| --- | --- |
| Latest 0.1.x release | Yes |
| Default branch | Yes, development only |
| Older snapshots | No |

Upgrade to the newest release before reporting a problem that may already be
fixed.

## Report A Vulnerability Privately

Do not open a public issue for a suspected vulnerability. Use
[GitHub private vulnerability reporting](https://github.com/chatcode-lab/stock-tui/security/advisories/new).
If that form is unavailable, contact the maintainers at `support@chatcode.dev`
with the subject `stock-tui security report`.

Include only what is necessary to reproduce and assess the problem:

- affected version, commit, operating system, and terminal;
- impact and realistic attack prerequisites;
- minimal reproduction steps or a small synthetic proof of concept;
- whether credentials, a local cache, or an opened news URL are involved;
- suggested remediation, if known.

Never send a real Alpaca key or secret. Do not attach a populated live-market
database. Redact provider account identifiers and use simulated records. If a
secret may have been exposed, revoke/rotate it in Alpaca immediately rather
than waiting for the project response.

Maintainers will acknowledge receipt when practical, validate the scope,
coordinate a fix and release, and credit the reporter if requested and safe.
Please allow a reasonable remediation window before public disclosure. This is
an open-source volunteer project and does not promise a response SLA or bug
bounty.

## Security Boundaries

The current application:

- runs as the invoking user and should not be installed or run with elevated
  privileges;
- reads Alpaca credentials from the process environment or a local `.env`;
- stores market data, news metadata, sync checkpoints, and favorites in a local
  SQLite file;
- makes HTTPS requests to configured provider/catalog base URLs;
- opens a selected news URL through the operating system's default browser;
- does not place orders and does not need brokerage trading authority for its
  intended market-data use.

Credential wrappers, debug redaction, and bounded error bodies reduce
accidental disclosure, but environment variables remain visible to processes
with sufficient local privileges. A compromised user account, terminal,
binary, dynamic loader, certificate store, proxy, or operating system is
outside the application's trust boundary.

Changing `STOCK_TUI_DATA_URL` or `STOCK_TUI_TRADING_URL` directs authenticated
requests to that host. Treat those overrides as security-sensitive and use
only endpoints you trust. A malicious catalog/provider can return crafted
names, headlines, URLs, timestamps, and numeric values; parsing and rendering
must remain bounded.

## Local Data Protection

The database does not intentionally store Alpaca credentials, but it can reveal
favorite tickers, accessed news, cached companies, and provider-derived market
history. Protect it with normal per-user filesystem permissions and private
backups. Do not place it in a public cloud-sync folder or repository.

SQLite WAL may leave adjacent `-wal` and `-shm` files while the app is open.
Secure and dispose of them with the main database. Stop the application before
making a file-level backup.

The application creates its config, cache, and data directories automatically.
Review their permissions on shared machines. A `.env` should be readable only
by its owner where the platform supports permissions.

## Dependency And Release Safety

Source builds should use the committed `Cargo.lock` and `--locked`. Download
prebuilt binaries only from the repository's GitHub Releases page and verify
published checksums when available. The existence of an archive on another
site is not proof that `chatcode-lab` produced it.

Security-sensitive dependency updates should include the reason, affected
surface, and normal format/lint/test results. Reports about a dependency should
identify whether the vulnerable code path is enabled or reachable in this
binary.

## What Is Not A Security Vulnerability

The following are important but normally belong in a public bug or data-quality
issue using demo/synthetic examples:

- stale, missing, delayed, or inaccurate prices;
- IEX values differing from consolidated SIP values;
- an outdated sector, rank, issuer name, or SEC ticker association;
- provider downtime, quota exhaustion, or an account entitlement error;
- terminal color/glyph differences without code execution or data exposure;
- investment losses or decisions based on displayed information.

A data issue becomes security-relevant when it enables a concrete attack such
as credential disclosure, arbitrary file access, command execution, terminal
escape injection, denial of service from a bounded payload, or opening a URL
without explicit user activation.

## Redistribution And Hosted Services

Redistributing third-party data without permission is a licensing incident,
not a supported feature. Alpaca states that ordinary API data cannot be
redistributed. Do not report a proposed shared-key proxy as a security fix and
do not include server credentials in this repository.

Any future fallback backend needs independent redistribution licenses,
server-side secret management, authentication, tenant isolation, rate limits,
abuse detection, encrypted transport, audit/retention controls, and a separate
threat model before production deployment.
