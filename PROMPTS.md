# Building stock-tui with Codex

`stock-tui` was developed iteratively with Codex in agentic terminal sessions
hosted by [chatcode.dev](https://chatcode.dev). This document connects the
human direction in those sessions to the repository changes that followed.
It is intended as a practical record of the feedback loop, not as a claim that
the project was created without human judgment or other automation.

The prompts below are lightly edited and abridged for grammar, clarity, and
readability while preserving their technical intent. Secrets, machine-local
paths, system instructions, raw tool output, and private provider data are
omitted. Attached StockTouch and work-in-progress screenshots are described
but not republished here. Every commit link is immutable; links into private
infrastructure repositories require collaborator access.

The chronology covers product-development prompts through the `v0.1.1`
release and the first post-release provider, onboarding, market-cap, and SEC
catalog pipeline, plus the first private development-provider prototype. It
excludes session-management instructions and this document's own editorial
requests.

## 1. Build a StockTouch-Inspired Market TUI

> Build a comprehensive analogue of StockTouch, the iOS app from several years
> ago, as a rich, `btop`-like terminal application with mouse input replacing
> touch interaction.
>
> Core concepts:
>
> - Represent stocks as color-coded rectangles, moving from bright red through
>   dark red and neutral gray to dark green and bright green according to the
>   intensity of each ticker's gain or loss over the selected period.
> - Divide the main screen into a 3x3 grid for nine economic sectors. Each
>   sector should contain a 10x10 grid representing its top 100 companies.
> - Let users open a sector, then open an individual ticker for a detailed
>   view.
> - Provide global range selectors for `1D`, `1W`, `1M`, `3M`, `6M`, `1Y`, and
>   `5Y`.
> - Provide ranking modes for market capitalization, gains, volume, and
>   alphabetical order.
> - Tint the ticker detail view according to performance and show price and
>   statistical values, a combined price and volume chart, and related news.
> - Search for companies by ticker or name.
> - Star and unstar tickers, expose a separate starred list, and emphasize
>   starred companies in shared views.
>
> Take inspiration from the best chart-heavy TUIs: use responsive layouts, a
> rich palette, keyboard and mouse input, and high-resolution character
> drawing with Braille or other suitable glyphs.
>
> Use Alpaca API credentials from `.env`. Cache historical data locally,
> probably in SQLite. On first launch, fetch the top 900 companies efficiently
> and respect free-tier rate limits. Later launches and periodic work should
> request only new data. Refresh sector membership when companies enter or
> leave a sector's top 100; data for companies outside those lists can be
> omitted unless it is already cached or retained for useful search
> visibility. Real-time data is unnecessary, but current-day data matters.
> Free US equity data is sufficient, with optional broader coverage when a
> user supplies an eligible key.
>
> Later, support a lightweight backend fallback for users without their own
> API keys, using centrally cached free-tier data fetched with project-owned
> credentials without exposing those credentials.
>
> Investigate a practical source for concise related news containing the date,
> headline, and source.
>
> Make this a polished public open-source project with comprehensive
> documentation, and publish it under `github.com/chatcode-labs/stock-tui` or
> another suitable name using the GitHub CLI. Use the supplied StockTouch
> screenshots as visual references.

**Committed changes**

- [`447fe68` - Initial Rust release][commit-447fe68]

**Summary**

The first commit created an initial end-to-end Rust/Ratatui application: the
3x3 market overview, sector and ticker drill-down, Alpaca provider, SQLite
cache and sync engine, search, favorites, news, demo mode, responsive
mouse/keyboard UI, tests, public documentation, and CI/release workflows. It
was published under the chosen `chatcode-lab/stock-tui` repository. The no-key
backend was documented as a future provider boundary because redistributing
market data requires suitable licenses; no credentials or populated provider
cache were embedded. Broader and non-US instruments also remain out of scope
pending a provider adapter with explicit symbol, currency, market-calendar,
entitlement, and licensing semantics.

## 2. Choose Rust or Go Instead of Python

> Why Python? Let us use Rust or Go instead. Which language is the better fit
> for this application?

**Committed changes**

- [`447fe68` - Initial Rust release][commit-447fe68]

**Summary**

The repository was implemented in Rust from its first commit, using Ratatui
and Crossterm for terminal rendering and input, Tokio for background work, and
`rusqlite` with bundled SQLite for local persistence. The README records why
Rust was selected over Go for precise cell/canvas rendering, explicit
concurrency, and single-binary distribution.

## 3. Report the Missing Rust Toolchain

> Running the demo currently fails because Cargo is not installed:
>
> ```text
> cargo run --release -- --demo
> Command 'cargo' not found
> ```
>
> The shell suggests installing Cargo or Rustup through the system package
> manager.

**Committed changes**

- No source commit was required; this was a session environment issue.
- [`447fe68` - Initial Rust release][commit-447fe68] documents source-build
  requirements.
- [`047df03` - Add ordered view navigation and release builds][commit-047df03]
  later expanded and published prebuilt packages that do not require Cargo.

**Summary**

The development environment was provisioned with the Rust toolchain without
changing application behavior. Cross-platform binary releases later removed
the Rust installation requirement for ordinary users.

## 4. First Data, Layout, Input, and Chart Review

> 1. Some sectors in the attached screenshots show a suspicious alternating
>    pattern of gains and losses. Is that genuine market data?
> 2. Make every visible ticker rectangle the same width and height whenever
>    possible, with balanced padding when space remains.
> 3. Mouse input does not work in this terminal. The Chatcode xterm client
>    forwards modern SGR mouse reports emitted through `term.onData`, but it
>    does not subscribe to `term.onBinary`, so legacy X10/default reports are
>    discarded. Ensure the app explicitly enables modern SGR mouse reporting.
> 4. The overview is visually uneven: tile sizes vary, strange one-character
>    rectangles appear in panel corners, and labels such as `EN057` and
>    `FN051` are synthetic IDs instead of real tickers.
> 5. Add discoverable keyboard hints for refresh, star/unstar, search, and
>    other actions. Present ranges clearly, for example as `1: 1D`,
>    `2: 1W`, and so on.
> 6. Improve the ticker chart. Fill the price area with a gradient similar to
>    Apple's Stocks app, and add price/value and date scales to the axes.

**Committed changes**

- [`89a09a2` - Refine demo data and terminal interaction][commit-89a09a2]

**Summary**

The fabricated ticker IDs and alternating demo values were replaced with real
SEC-catalog identities and independently seeded simulated returns. The app
began requesting SGR `1003`/`1006` mouse reports, centered uniform heatmap
cells with balanced padding, expanded visible keyboard and status guidance, and
added filled price charts with labeled value and date axes. Rendering and
terminal-input regression tests were added with the changes.

## 5. Use Live Data and Align Mouse Behavior With Keyboard Selection

> 1. Why use demo mode when working Alpaca API keys are already available?
> 2. On the overview, hovering should highlight only an entire sector,
>    matching arrow-key selection. Do not make a per-ticker star action appear
>    and disappear on hover because that causes layout jumping.
> 3. In a sector view, hovering should select a ticker exactly as arrow
>    navigation does and update its summary at the top.
> 4. The detail chart is improved but still too pixelated. Use more gradual
>    gradient levels, thinner Braille-based lines, and eliminate
>    duplicate-looking traces. Make volume more expressive as well.
> 5. Put a clear star indicator in the ticker headline. Emphasize starred
>    tickers in overview and sector views with a star, border, bold text, or
>    another space-efficient treatment.
> 6. When returning from ticker detail to a sector, preserve the previously
>    selected ticker instead of resetting to the first one.
> 7. If opening a selected news article fails, copy its URL through the
>    terminal clipboard mechanism so Chatcode can offer to open it in a
>    separate tab.
>
> Also clarify whether the displayed news is live/provider-sourced or
> simulated.

**Committed changes**

- [`4ecea42` - Refine selection and ticker charts][commit-4ecea42]
- [`447fe68` - Initial Rust release][commit-447fe68] contains the original
  Alpaca news adapter.

**Summary**

The README made credentialed live startup the primary run path; the existing
resolver already selected Alpaca whenever both credentials were configured.
Mouse hover and keyboard focus share the same sector, ticker, and news
selection state; back navigation preserves the originating ticker; and
favorites are emphasized without layout shifts. The detail view gained thin
Braille price rendering, smoother fill, higher-resolution volume rendering,
and an OSC 52 clipboard fallback for news URLs. Live mode uses Alpaca news,
while demo headlines remain clearly labeled as simulated.

## 6. Clarify Refresh vs. Sync and Simplify the Chart Cursor

> What is the difference between Refresh and Sync? Do either of them run
> automatically on an interval?
>
> The chart is cleaner but still shows one-character horizontal shifts. Volume
> bars have small gaps and uneven bottom edges. Remove the unexplained dashed
> gray horizontal line. Hover or keyboard inspection should draw only a
> vertical cursor with a visible marker at its intersection with the price
> trace; a horizontal crosshair is unnecessary.
>
> Can the chart edge be made less chunky? Consider a short reverse gradient
> fading to black outside the trace.

**Committed changes**

- [`ce2dfdc` - Simplify charts and clarify refresh status][commit-ce2dfdc]

**Summary**

The UI and documentation separated `r`, an immediate broad-market snapshot
refresh, from `S`, a read-only data-status panel. Live refresh runs at startup
and on a five-minute cadence by default, and manual refresh restarts the timer.
The chart dropped the horizontal crosshair and extra reference treatment, kept
a vertical cursor and intersection marker, softened the fill boundary with a
short exterior fade, and made the volume columns contiguous.

## 7. Improve Focus Contrast on Bright Tiles

> The light-blue hover/selection color is difficult to read on bright red or
> green ticker tiles. Invert or darken the focus treatment on bright
> backgrounds so it always has sufficient contrast.

**Committed changes**

- [`14c5735` - Improve heatmap focus contrast][commit-14c5735]

**Summary**

Heatmap focus became luminance- and contrast-aware: selected text is dark on
bright gain/loss colors and light on dark backgrounds. Palette and integration
tests cover both ends of the heat scale.

## 8. Fix Inconsistent Foregrounds and Remaining Chart Gaps

> In the attached screenshot, `OLPX` and `MMLP` unexpectedly use a gray
> foreground. Fix that inconsistent text color.
>
> The chart still has one-character shifts: even the indicator is not
> straight. Volume bars still show horizontal gaps. Investigate and correct
> both issues.

**Committed changes**

- [`79bcd5a` - Fix stale contrast and chart seams][commit-79bcd5a]

**Summary**

Stale tickers retained the same contrast-aware foreground as fresh tickers and
used an underline as the freshness cue instead of low-contrast gray. The chart
cursor moved to one straight terminal-cell column, while fully occupied volume
cells used background fill to reduce font-dependent seams between glyphs.
Later feedback showed that the browser terminal still exposed alignment
artifacts requiring further refinement.

## 9. Explain Staleness and Expand Navigation, Ranges, Metrics, and Benchmarks

> Why does ticker data become stale? Should not every retained ticker be
> updated?
>
> 1. In ticker detail, use Up/Down to select news articles and Left/Right to
>    move the chart cursor.
> 2. Add ranges in this order: `1D`, `1W`, `1M`, `3M`, `6M`, `1Y`, `2Y`,
>    `5Y`, `10Y`, and `ALL`; bind `0` to `ALL`.
> 3. Chart-fill color must depend only on vertical position and the
>    dark-to-light value range, never on the X coordinate.
> 4. Show absolute price gain/loss alongside the percentage change.
> 5. Add uniform direct sector shortcuts using `c`, `s`, `h`, `e`, `t`, `f`,
>    `i`, `m`, and `u` for Consumer, Services, Healthcare, Energy,
>    Technology, Financial, Industrial, Materials, and Utilities. A modifier
>    other than Ctrl is acceptable if it avoids conflicts, but keep the scheme
>    uniform.
> 6. Add S&P 500, Dow, and Nasdaq status to the bottom of the overview. Make
>    these benchmarks selectable and clickable, with detailed charts and news
>    when the provider supports them.

**Committed changes**

- [`cf7983a` - Expand market ranges and benchmark navigation][commit-cf7983a]

**Summary**

The app added `2Y`, `10Y`, and `ALL`; absolute and relative gains; Y-only fill
intensity; independent chart/news arrow navigation; sector shortcuts; and
selectable `SPY`, `DIA`, and `QQQ` benchmark proxies with detail views. Sync
planning expanded to reconcile active Alpaca assets, refresh retained
candidates and benchmarks, and maintain daily plus weekly history. The docs
also explain that an IEX symbol can still appear stale after a successful
refresh when the free feed has no recent trade for it.

## 10. Align Benchmarks and Improve Range-Switching Performance

> Align the S&P 500, Dow, and Nasdaq borders with the three sector columns, and
> keep the benchmark strip constrained to the market grid rather than the full
> terminal width.
>
> Reference values and horizontal grid lines still exhibit one-symbol shifts.
> Volume bars retain small horizontal gaps and a slightly nonuniform bottom
> edge, although this may be browser rendering.
>
> Switching ranges takes several seconds in every view. Look for an easy
> performance improvement, such as a suitable database index or avoiding
> repeated work.

**Committed changes**

- [`e7fc9ba` - Polish chart rendering and range latency][commit-e7fc9ba]

**Summary**

The benchmark footer adopted the same centered three-column geometry as the
sector grid. Cursor and solid-volume geometry moved to terminal-cell
backgrounds in an attempt to isolate them from font shaping, and range
selection stopped scanning all distinct bar timeframes. Indexed
`(symbol, timeframe)` availability probes removed the repeated full-history
scan behind the multi-second range-switching delay.

## 11. Restore the Better Cursor and Volume Detail

> The latest chart change made rendering worse:
>
> 1. The half-symbol shift remains. Determine whether it is fundamentally an
>    xterm/browser font-rendering issue.
> 2. The thick cursor is worse than the earlier Braille cursor.
> 3. The volume chart lost information: its previous color was more uniform
>    and its heights had more variation. Restore that useful detail without
>    reintroducing gaps.
> 4. Draw Y-axis reference prices over the chart rather than reserving space
>    before the plot. Axis labels should have priority over the plot, while the
>    active cursor should render above both.
> 5. `Option`/`Alt` plus sector-letter shortcuts do not work in this xterm
>    environment.

**Committed changes**

- [`5d2df0f` - Refine chart axes and sector shortcuts][commit-5d2df0f]

**Summary**

The chart restored a thin fixed Braille-subcolumn cursor, seam-free full volume
cells with eighth-cell fractional caps, and price labels painted over the plot
with explicit layer priority. To support terminals such as the reported host,
which did not reliably transmit Alt/Meta combinations, the app added the
portable `g` followed by a sector letter chord while retaining modifier
shortcuts where supported.

## 12. Eliminate the Cursor's Cumulative Horizontal Drift

> The remaining horizontal shift is cumulative: it is almost absent with the
> cursor on the left, visible near the middle, and much larger on the right.
> This suggests the grid-line glyphs or font advances are accumulating error.
>
> If Braille grid lines cannot be fixed reliably, replace them with another
> small dot character or a more stable Braille pattern. Also restore a clear,
> noticeable marker where the cursor intersects the price trace.

**Committed changes**

- [`1868be2` - Stabilize chart guides and cursor marker][commit-1868be2]

**Summary**

Long Braille guide runs were replaced by one middle-dot glyph per terminal
cell to avoid suspected fallback-font advance errors and remove the
accumulating browser-hosted-row artifact. A high-contrast cyan marker again
identifies the cursor/trace intersection, with regression tests for the guide
and cursor layers.

## 13. Add Ordered Previous/Next Navigation and Publish a Release

> Add simple hotkeys for previous and next navigation:
>
> - In sector views, move to the previous or next sector.
> - In ticker detail, move to the previous or next ticker using the current
>   visible ordering.
>
> Show the current ticker rank in the top detail summary so users can see their
> position while moving through the ordered list.
>
> Then push the changes and publish a GitHub release with builds for all major
> platforms.

**Committed changes**

- [`047df03` - Add ordered view navigation and release builds][commit-047df03]
- [`v0.1.0` release][release-v0.1.0]

**Summary**

The `p` and `n` keys gained wraparound navigation through sectors and through
the exact active sector, Favorites, or benchmark ordering. Detail headers show
the current one-based rank and sort mode. The existing release workflow
expanded from three targets to five and produced version smoke-tested,
checksummed archives for Linux x86_64/ARM64, macOS Intel/Apple Silicon, and
Windows x86_64; the validated commit was published as `v0.1.0`.

## 14. Review PR #6, Center the Cursor Glyph, and Add README Screenshots

> Review and address pull request #6.
>
> Change the vertical chart cursor to a centered, single-middle-dot pattern.
> The current Braille cursor alternates between a left-aligned and
> right-aligned column of three dots, which looks uneven.
>
> Render project-owned screenshots of the market overview, a sector view, and
> one or more ticker charts, and add them to the main README.

**Committed changes**

- [`d046cb9`][commit-d046cb9] - Bump the rust-dependencies group across 1
  directory with 10 updates
- [`65c194a`][commit-65c194a] - Merge pull request #6 from
  chatcode-lab/dependabot/cargo/rust-dependencies-f4d9375085
- [`95f20b6` - Center chart cursor and prepare v0.1.1][commit-95f20b6]
- [Pull request #6][pr-6]

**Summary**

The compatible dependency lockfile update was reviewed, validated across
platforms and Rust 1.95, and merged. Every cursor row then switched to one
centered middle dot while preserving an inverse cyan price intersection.
Deterministic demo captures of the overview, Technology sector, and ticker
detail screens were generated, labeled as simulated, and embedded in the
README. The package and changelog were prepared for `v0.1.1`.

## 15. Ship v0.1.1

> When ready, ship `v0.1.1`.

**Committed changes**

- [`95f20b6` - Center chart cursor and prepare v0.1.1][commit-95f20b6]
- [`v0.1.1` release][release-v0.1.1]

**Summary**

The release was gated on formatting, Clippy, the complete test suite, MSRV and
dependency-policy checks, plus a non-publishing five-platform release dry run.
The annotated tag points to `95f20b6`; GitHub published five native archives
and `SHA256SUMS`. The public assets were downloaded again, their checksums and
archive contents were verified, and the Linux x86_64 binary reported
`stock-tui 0.1.1`.

## 16. Replace Previous/Next Keys and Explain Production Credentials

> Change the sibling-navigation controls: use `Space` for next instead of `n`,
> `Backspace` for previous instead of `p`, and keep `Esc` as the command for
> going up one level.
>
> Also explain how a production installation should set its API key. Can it
> use the same `.env` file?

**Committed changes**

- [`3e89a7f` - Add live data onboarding and provider catalog pipeline][commit-3e89a7f]

**Summary**

The event handling, visible hints, and tests moved ordered navigation to
`Space` and `Backspace`, leaving `Esc` as the only route-up action. Installed
binaries can still read a private `.env` from the launch directory or its
parents, or receive credentials through exported environment variables.
First-run onboarding later added a platform configuration-directory
`credentials.env`; `--print-config` reports the resolved non-secret paths.

## 17. Locate Application Data on macOS

> Where does the application store its data on macOS?

**Committed changes**

- [`3e89a7f` - Add live data onboarding and provider catalog pipeline][commit-3e89a7f]

**Summary**

The cache documentation now lists the platform paths and recommends
`stock-tui --print-config` as the authoritative answer. The default macOS live
database is
`~/Library/Application Support/com.chatcode-lab.stock-tui/market.sqlite3`;
configuration, downloaded catalog, and logs use the platform-specific config
and cache directories instead of being mixed into the data directory.

## 18. Diagnose Broken macOS Charts and Separate Demo Data

> The chart looks broken on my Mac even though it renders correctly in the
> Linux xterm environment.
>
> I may have launched once without an API key and then restarted with the key
> configured, causing simulated and live observations to mix. We should clear
> test data in that situation.

**Committed changes**

- [`3e89a7f` - Add live data onboarding and provider catalog pipeline][commit-3e89a7f]

**Summary**

The investigation identified cache provenance as a separate risk from terminal
glyph rendering. Demo and live runs now default to different databases,
`demo.sqlite3` and `market.sqlite3`. On upgrade, a legacy demo-to-live
transition removes simulated bars, snapshots, news, memberships, and demo
checkpoints while preserving favorites and already fetched live-provider
records. Demo generation remains explicitly resettable with `--reset-demo`.

## 19. Consider a Shared Cloudflare Market-Data API

> Build a thin Cloudflare API that periodically fetches the required Alpaca
> data, stores it in D1, caches responses, and lets clients run without demo
> mode or personal keys. Keep personal Alpaca credentials as a compatibility
> option and expose the service below a `chatcode.dev` domain.
>
> Prepare the Wrangler configuration and I will authorize it. D1 is the intended
> database. Before proceeding, determine whether Alpaca permits this for a
> non-commercial open-source application or offers a free allowance.

**Committed changes**

- No shared-key market-data service was committed from this proposal.
- [`3e89a7f` - Add live data onboarding and provider catalog pipeline][commit-3e89a7f]
  records the resulting licensing boundary and limits Cloudflare publication
  to SEC-derived catalog data.

**Summary**

Review of Alpaca's published terms and support guidance found that an ordinary
personal plan does not grant redistribution rights. Free access, low request
volume, and non-commercial intent do not by themselves authorize serving the
same market observations to arbitrary clients. The project therefore did not
put a maintainer key behind a public proxy. The later Cloudflare work publishes
only the independently derived SEC issuer catalog, not provider prices, bars,
volume, news, or credentials.

## 20. Cancel the Proxy and Document Bring-Your-Own-Key Licensing

> Cancel the shared API layer; I do not want a legal conflict. Document how a
> user can obtain a personal free key instead.
>
> How should we ask Alpaca whether a free or sponsored license is available
> for this open-source application?

**Committed changes**

- [`3e89a7f` - Add live data onboarding and provider catalog pipeline][commit-3e89a7f]

**Summary**

Bring-your-own-key became the documented live-data model. The setup guide
covers a free Paper Trading account, the Basic/IEX feed, one-time secret
handling, local dotenv use, and credential redaction. The provider document
adds a public-display and redistribution inquiry checklist plus a concise
request template covering fields, derived displays, caching, retention,
audience, attribution, extraction controls, and separate news rights. It also
states that a plan upgrade, rate-limit increase, marketing reply, or OAuth
approval is not a redistribution license.

## 21. Add Interactive Credential Onboarding

> If no valid API key is configured, show a registration link, try to open it
> or copy it to the clipboard if opening fails, then accept the key and secret
> without displaying them. Store the credentials locally and start the normal
> application.
>
> Keep storage simple: a private raw `.env`-style file in the user's home or
> application directory is sufficient.
>
> Do not open the registration page automatically. Wait for input:
> `Enter` opens it, `c` copies it, and `Esc` skips opening and continues to key
> entry. Also provide `d` so the user can explicitly start the demo.

**Committed changes**

- [`3e89a7f` - Add live data onboarding and provider catalog pipeline][commit-3e89a7f]

**Summary**

A pre-TUI onboarding flow now appears only for an online Alpaca launch without
a complete credential pair. It waits for an explicit open, copy, demo, or
continue action; uses OSC 52 as the terminal clipboard path; reads both
credential fields with echo disabled; validates them against the provider; and
writes a mode-restricted `credentials.env` below the platform config
directory. Existing process or working-directory dotenv credentials retain
precedence, and partial pairs are never combined with stored values.

## 22. Polish Onboarding Links, Startup Status, and CLI Help

> Encode the registration URL so terminals render it as a highlighted,
> clickable link.
>
> Credential validation and initial cache work can take noticeable time after
> the values are saved, so print status before the main UI starts.
>
> Remove the old `--demo` hint from the onboarding text now that `d` is
> available there, but keep `--demo` as a command-line option. Add `-h` and
> `--help`.

**Committed changes**

- [`3e89a7f` - Add live data onboarding and provider catalog pipeline][commit-3e89a7f]

**Summary**

The signup destination is emitted as an OSC 8 hyperlink with readable fallback
text. Onboarding and pre-TUI startup report validation, persistence, catalog,
database, and synchronization progress instead of appearing frozen.
Credential prompts advertise the direct `d` choice without redundant launch
instructions, while the Clap CLI exposes standard `-h`/`--help`,
`-V`/`--version`, provider, catalog, database, feed, demo, offline, and
redacted configuration options.

## 23. Investigate Alphabet's Missing GOOG Ticker

> Why is `GOOG` absent from the Technology sector? How are companies assigned
> to sectors and ordered?

**Committed changes**

- [`3e89a7f` - Add live data onboarding and provider catalog pipeline][commit-3e89a7f]

**Summary**

The investigation documented that the catalog is issuer-based: SEC CIK
identity is mapped from the newest eligible SIC observation into one of the
nine legacy sectors, and one canonical exchange ticker represents each issuer.
The original source-order rule selected Alphabet's `GOOGL`. Canonical selection
now safely prefers a concise common-equity base such as `GOOG`, while rejecting
preferred, warrant, unit, right, note, and economically incompatible explicit
class substitutions. Sector membership initially uses SEC public float as a
numeric size proxy and is recomputed from estimated market cap when valid share
and price data become available.

## 24. Fill the Market-Capitalization Gap

> Why is market capitalization unavailable? Can another free source supply it,
> or can we calculate it? Missing market cap may exclude major companies such
> as Alphabet and is probably not isolated to one issuer.

**Committed changes**

- [`3e89a7f` - Add live data onboarding and provider catalog pipeline][commit-3e89a7f]

**Summary**

Alpaca's basic asset response does not provide a complete market-cap field, and
SEC `EntityPublicFloat` is a ranking proxy rather than market capitalization.
The catalog builder now extracts common-share estimates from SEC Frames and
Financial Statement Data Sets with source, date, method, confidence, and
component provenance. Runtime calculates an estimated ordinary-equity market
cap only when it can multiply that reviewed share estimate by a current
provider price. Proxy-only companies remain eligible for sector ranking, but
the UI does not mislabel public float as market cap.

## 25. Model and Validate the Market-Cap Calculation

> Model the calculation first and compare its results with public reference
> values such as Yahoo Finance.
>
> You may try parsing Yahoo's key-statistics pages using the supplied browser
> session cookie, in full or in part.
>
> Agreed: proceed with the auditable model instead of making the client depend
> on that scrape.

**Committed changes**

- [`3e89a7f` - Add live data onboarding and provider catalog pipeline][commit-3e89a7f]

**Summary**

The supplied session data was not stored or committed, and authenticated page
scraping was rejected as a brittle client dependency. The implemented model
uses official SEC facts and a contemporaneous provider price. It distinguishes
point-in-time totals from lower-confidence weighted-average fallbacks,
excludes diluted and preferred securities, records all assumptions, and fails
closed on unknown multi-class structures. Reviewed policies cover equal
economic classes, explicit conversions, and filer-reported equivalents where a
naive share sum or canonical-price multiplication would be misleading.

## 26. Estimate How Quickly a New IPO Appears

> If a company such as OpenAI completes an IPO, how quickly can the current
> model discover, rank, and display it correctly?

**Committed changes**

- [`3e89a7f` - Add live data onboarding and provider catalog pipeline][commit-3e89a7f]

**Summary**

Discovery depends on upstream publication rather than a hard-coded company
list. A new issuer must appear in the SEC ticker association data, receive an
eligible SIC and ranking/share facts, and be active in the selected market-data
provider. The daily catalog job can publish it after those inputs exist, and
clients recheck the compact catalog every 12 hours by default. A newly listed
company can remain search-only or proxy-ranked until sufficient fundamentals
and price data exist; the builder does not invent missing market cap or
silently guess an unreviewed share structure.

## 27. Keep the Python Catalog Builder Off Client Machines

> How does the Python catalog-builder flow work? Is it run while building a
> release? Could the catalog be supplied through an API? I do not want Python
> to become a client-side dependency.

**Committed changes**

- [`3e89a7f` - Add live data onboarding and provider catalog pipeline][commit-3e89a7f]

**Summary**

`tools/build_sec_catalog.py` is a maintainer and CI tool, never a runtime
dependency. It downloads official SEC inputs, creates a verbose audit catalog,
and projects only the Rust-consumed fields into canonical gzip JSON with a
checksum manifest. Release jobs download one validated compact catalog before
building all platform binaries, while the Rust client can refresh a newer copy
from the catalog endpoint and cache it locally. Source builds and outages still
have a checked-in, validated fallback.

## 28. Measure Catalog Automation on GitHub Actions

> How expensive is a catalog update? Can it run on GitHub Actions for free?

**Committed changes**

- [`3e89a7f` - Add live data onboarding and provider catalog pipeline][commit-3e89a7f]

**Summary**

A measured warm build took roughly 72 seconds and about 205 MB of peak memory;
the cold SEC source cache was about 154 MB. That cache contains quarterly bulk
archives and is not shipped to users. The public repository can run the job on
standard GitHub-hosted Actions. The committed workflow restores immutable SEC
downloads, explicitly invalidates mutable ticker and Frame inputs, validates
fixtures and catalog invariants, packages the compact artifact, and supports
both a daily off-peak schedule and manual dispatch.

## 29. Generalize Providers and Automate a Compact Catalog

> Make these broader architectural changes:
>
> 1. Keep releases independent of the 150-200 MB builder cache. Use a compact
>    15-20 MB fallback catalog at most, or download it on demand.
> 2. Put a provider-neutral abstraction in front of market APIs. Alpaca should
>    become one configurable adapter selected by command-line arguments or a
>    config file.
> 3. Build the SEC catalog daily, perhaps twice daily, in GitHub Actions.
>    Avoid requiring Python on client machines.
> 4. Reconsider a private default API backed by Alpaca, D1, and edge caching,
>    hiding the upstream provider and serving enriched catalog data to
>    `stock-tui` clients.
>
> What are the tradeoffs?

**Committed changes**

- [`3e89a7f` - Add live data onboarding and provider catalog pipeline][commit-3e89a7f]

**Summary**

The size premise was corrected: approximately 154 MB belonged to the
maintainer's SEC download cache, while the checked-in audit JSON was about
3.2 MB and its compact runtime projection was below 1 MB before transport
compression and roughly 86 KB as deterministic gzip. The project keeps that
small embedded fallback and obtains freshness remotely.

The runtime now exposes provider-neutral asset, market-data, and optional-news
capabilities with provider-specific configuration namespaces; Alpaca is one
adapter rather than a type embedded throughout synchronization and UI code. A
versioned HTTP adapter and public contract define how a separately authorized
service could integrate, but no shared-key service is claimed as deployed or
licensed. The request to conceal Alpaca provenance and redistribute its data
was rejected: transport indirection and removed attribution would not create
redistribution rights.

The catalog job was set to daily rather than twice daily because SEC
fundamental inputs do not benefit materially from a twelve-hour build cadence.
It publishes only independently derived SEC metadata and keeps the large source
cache inside CI.

## 30. Publish Fresh Catalogs Through Cloudflare R2

> Forget the market-data proxy and implement the remaining architecture.
> Do not commit every freshly generated catalog to the repository; publish it
> to Cloudflare R2 instead. Configure the infrastructure with Wrangler and
> expose a stable static URL such as
> `lab.chatcode.dev/stock/sec-catalog.tar.gz`.

**Committed changes**

- [`3e89a7f` - Add live data onboarding and provider catalog pipeline][commit-3e89a7f]

**Summary**

The committed Cloudflare tooling provisions a dedicated `stock-tui-catalog` R2
bucket and custom-domain layout under `stock.chatcode.dev`. A gated daily
workflow builds and validates the audit catalog, packages deterministic
single-file gzip JSON instead of adding an unnecessary tar layer, writes
compressed and uncompressed SHA-256 metadata, uploads immutable versioned
objects, and updates short-cache stable catalog and manifest keys. Fresh audit
catalogs remain ephemeral CI artifacts rather than creating repository commit
churn.

At startup, Rust renders from the newest valid embedded or cached catalog and
checks the R2 object in the background, so first paint is not network-bound.
Release builds fetch one validated current catalog for every target, keeping
binary contents consistent while preserving the repository snapshot for source
builds and outages. Wrangler provisioning and publication are documented and
opt-in; the workflow requires explicitly configured Cloudflare credentials,
SEC contact identity, and enablement variable.

## 31. Prototype a Private Development Provider and Prefer Concise Tickers

> Provide the Wrangler authentication commands when Cloudflare access is
> needed.
>
> Implement the proxy API as a private, development-only alternative provider.
> Reserve a base such as `stock.chatcode.dev/api`, require no client
> authentication, and keep the implementation in a separate private
> `chatcode-lab/stock-api` repository. Enrich its normalized responses with
> proper market-cap estimates.
>
> Document the provider boundary so users can implement compatible services or
> adapt other market-data providers.
>
> Prefer `GOOG` over `GOOGL` as Alphabet's canonical ticker, and favor symbols
> of four characters or fewer wherever that substitution is economically safe
> because overview tiles have limited space.

**Committed changes**

- [`3e89a7f` - Add live data onboarding and provider catalog pipeline][commit-3e89a7f]
- [`a67e660` - Add private stock data worker][private-commit-a67e660]

**Summary**

The Rust client gained a credential-free, provider-neutral `stock-api` adapter
selected independently from Alpaca. Its versioned HTTP contract covers active
assets, snapshots, adjusted bars, optional news, pagination, errors, limits,
and cache semantics; user-defined or third-party services can implement that
contract without changing the TUI. Snapshot market caps are accepted only as
positive finite values and fill companies lacking a stronger local SEC
share-based estimate.

The separate private Worker implements the same contract, an internal
`MarketDataProvider` interface, strict upstream normalization, exact-response
R2 caching, bounded D1 observations and opaque page tokens, and market-cap
estimates carrying price, share, calculation, method, and confidence
provenance. Local development needs no downstream authentication. Committed
configuration disables upstream fetching, deployment, preview URLs,
`workers.dev`, schedules, and public-distribution acknowledgement; no Worker,
D1 database, API route, or market-data cache was deployed because a private
Git repository does not make an unauthenticated internet endpoint private or
grant redistribution rights.

The SEC catalog canonicalizer now selects `GOOG` for Alphabet and applies a
conservative concise-symbol rule across issuers. It rejects derivatives and
does not collapse explicit share classes with different economics merely to
save tile width. Cloudflare catalog provisioning uses a temporary token
supplied only through the operator's environment; credentials and account
identifiers are never copied into the repositories or session transcript.

## Maintenance Outside the Prompt Loop

Not every repository change originated in a product prompt. GitHub Actions
pin updates in [`1811bd2`][commit-1811bd2] were maintenance work, and the
dependency changes in [`d046cb9`][commit-d046cb9] were authored by Dependabot
before the later request to review and merge pull request #6. The commit map
above distinguishes those automated contributions from the user-directed
implementation work.

## What the Process Demonstrates

- A detailed product brief can become a working, documented system spanning a
  terminal UI, provider integration, persistence, synchronization, tests, and
  release automation.
- Screenshot-driven feedback can guide small, test-backed refinements to
  layout, color contrast, chart composition, and interaction behavior.
- Agentic sessions can diagnose environment-specific details such as SGR mouse
  transport, terminal glyph shaping, clipboard escape sequences, and database
  query latency.
- Product decisions can remain explicit about data provenance, simulation,
  provider limits, licensing, cache safety, and unimplemented future work.
- A licensing review can stop an unsafe backend design before deployment while
  still producing reusable provider boundaries and independently distributable
  catalog infrastructure.
- Maintainer-only Python and Cloudflare tooling can update a versioned,
  checksummed data artifact without adding a language runtime or large source
  cache to the Rust client.
- The same session can carry changes through implementation, review, CI,
  cross-platform packaging, public release, and post-release artifact
  verification.

[commit-447fe68]: https://github.com/chatcode-lab/stock-tui/commit/447fe682d972234ee8b5f9f471a5bee966e2808d
[commit-1811bd2]: https://github.com/chatcode-lab/stock-tui/commit/1811bd2388cad9a42c1befbf92b82696212ac00a
[commit-89a09a2]: https://github.com/chatcode-lab/stock-tui/commit/89a09a2ea343ba51a66ab2a58de18d88a7d73964
[commit-4ecea42]: https://github.com/chatcode-lab/stock-tui/commit/4ecea42615aeac2542b42d6ef16193d952dd6456
[commit-ce2dfdc]: https://github.com/chatcode-lab/stock-tui/commit/ce2dfdc55b119cb9d82986248fafec61de72e5e9
[commit-14c5735]: https://github.com/chatcode-lab/stock-tui/commit/14c57351becd8d9abdffa23dda05c184ede61d26
[commit-79bcd5a]: https://github.com/chatcode-lab/stock-tui/commit/79bcd5a1ea7b8871b503781b8e41119f803204f0
[commit-cf7983a]: https://github.com/chatcode-lab/stock-tui/commit/cf7983a8bc4e229497b1856024ee2fe52a8ae37d
[commit-e7fc9ba]: https://github.com/chatcode-lab/stock-tui/commit/e7fc9bab4e2f97284cb0e36b87eb6c3ed4579dd3
[commit-5d2df0f]: https://github.com/chatcode-lab/stock-tui/commit/5d2df0fdcf606d32a194ecd6da68af4db17250c5
[commit-1868be2]: https://github.com/chatcode-lab/stock-tui/commit/1868be2d9a9ab54189945cde527fa911a58816e6
[commit-047df03]: https://github.com/chatcode-lab/stock-tui/commit/047df03ea4c1b5f28cc43403bd2601ad50185e31
[commit-d046cb9]: https://github.com/chatcode-lab/stock-tui/commit/d046cb90955396830a8a072fc2058fd7e5612354
[commit-65c194a]: https://github.com/chatcode-lab/stock-tui/commit/65c194a0e67c146c7d44fdbb89f5865dbadd565e
[commit-95f20b6]: https://github.com/chatcode-lab/stock-tui/commit/95f20b631921053ee84a47a41b6b0ceefd416b57
[commit-3e89a7f]: https://github.com/chatcode-lab/stock-tui/commit/3e89a7f9134f2e7246f8bb9a55a30cff4c74d936
[private-commit-a67e660]: https://github.com/chatcode-lab/stock-api/commit/a67e660f53e754c8e2bf45ba3b3a1ea8ab5fbd42
[pr-6]: https://github.com/chatcode-lab/stock-tui/pull/6
[release-v0.1.0]: https://github.com/chatcode-lab/stock-tui/releases/tag/v0.1.0
[release-v0.1.1]: https://github.com/chatcode-lab/stock-tui/releases/tag/v0.1.1
