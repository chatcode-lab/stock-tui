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

The chronology covers product-development prompts through the `v0.2.7`
release and subsequent UI refinements, including the provider, onboarding,
market-cap, SEC catalog, private development-provider, UI refinement, and
signed distribution work. It excludes session-management instructions and
this document's own editorial requests.

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

## 32. Avoid Remote Wrangler OAuth

> Wrangler browser authentication fails in the remote terminal. Cloudflare's
> OAuth page returns a bot-challenge response and `403 Forbidden` instead of
> JSON, while the authorization callback targets `localhost` on the machine
> running Wrangler.

**Committed changes**

- [`0b6e198` - Use API tokens for remote Wrangler setup][commit-0b6e198]

**Summary**

The one-time R2 instructions no longer recommend `wrangler login` from a
remote session. They direct the operator to create a narrowly scoped
Cloudflare REST API token in a normal browser. The provisioning script securely
prompts for that token without echoing it, asks for validated account and zone
IDs, and verifies R2 access before attempting to create resources. Missing
credentials now fail immediately in non-interactive runs instead of silently
falling through to browser OAuth; no token is written to the repository,
Wrangler OAuth storage, or shell history.

## 33. Secure, Cache, and Route the Private Test Provider

> Complete the Cloudflare setup directly and allow the rotated Wrangler API
> token to be entered again.
>
> Optimize and deploy the Worker on the reserved `/api` route. Confirm whether
> it has request rate limits and cached responses, make the Rust client usable
> with this API, and audit the catalog for other ticker symbols longer than four
> characters.

**Committed changes**

- [`ce13883` - Add private API auth and reviewed ticker alias][commit-ce13883]
- [`59bd27f` - Deploy the private authenticated stock gateway][private-commit-59bd27f]
- [`75e605b` - Fix deploy workflow temporary config path][private-commit-75e605b]
- [`210886a` - Optimize cached response delivery][private-commit-210886a]
- [`31901f8` - Fail fast on invalid upstream credentials][private-commit-31901f8]
- [`6bf23bb` - Preserve the Worker fetch receiver][private-commit-6bf23bb]
- [`cc1f6ca` - Use Worker-compatible manual redirects][private-commit-cc1f6ca]

**Summary**

The development service changed from the earlier proposed unauthenticated
shape to a fail-closed private gateway. Every versioned route requires an
environment-only bearer token outside loopback. Cloudflare's Rate Limit binding
allows 120 authenticated requests per 60 seconds per SHA-256 token fingerprint;
cache hits still consume this client quota. Complete responses remain cached in
R2 for five minutes to 24 hours depending on endpoint, bounded D1 records cover
fresh snapshots, catalog fundamentals, and pagination state, and stale data is
served only within endpoint-specific error windows. The deployment workflow
maintains a 30-day response-object lifecycle without replacing unrelated R2
rules. Validated cache keys canonicalize ticker order, equivalent timestamps,
and endpoint defaults; a hit streams the stored JSON body without parsing and
serializing large historical pages again.

The private repository gained a dispatch-only, idempotent provisioning and
deployment workflow. It resolves or creates D1 and the response-cache bucket,
requires the independently published catalog bucket, writes the otherwise
uncommitted D1 identifier and enabled private gates into a mode-`0600`
temporary config, migrates D1, uploads secrets through standard input, and
routes only `stock.chatcode.dev/api/*`. Its smoke test requires anonymous
rejection, authenticated health, a live SEC-enriched `NVDA` snapshot, and a
repeat R2 cache hit. Dependency installation and tests do not receive
production secrets; `workers.dev`, preview URLs, and scheduled warming remain
disabled.

The Rust adapter accepts the service token only through
`STOCK_TUI_STOCK_API_TOKEN`, never through CLI or TOML. It marks the header
sensitive, disables redirects, omits token presence from printed configuration
and debug output, and never shares it with the Alpaca adapter.

The freshly published catalog initially contained eleven five-character
canonical symbols. Review found only one sound shorter listing: Molson Coors
now uses `TAP` instead of `TAP-A`, scoped to that issuer while `TAP` remains
listed. Its classes share the reviewed economic rights and one-for-one
conversion but retain different voting rights. The ten remaining long symbols
are valid listings or explicit share classes and are never truncated into
nonexistent tickers.

The live deployment exposed two edge-runtime assumptions that Node fixtures
did not: a stored host `fetch` function must retain direct-call semantics, and
Cloudflare Workerd rejects `redirect: "error"`. A local Workerd reproduction
isolated the latter before the provider switched to manual redirect handling,
which never forwards credentials. The deployment workflow now validates
upstream access before provisioning and reports only stable error codes. Its
final smoke test passed authenticated health, live `NVDA` pricing, SEC
market-cap enrichment, and a repeat R2 response-cache hit.

## 34. Configure the HTTP Provider and Sign macOS Builds

> Make the Stock API provider values configurable in the application
> configuration file and clarify whether that provider is hardcoded.
>
> Review and address the open Dependabot pull requests. Then make the macOS
> release builds signed and notarized; signing credentials can be supplied
> when the workflow is ready.
>
> Compare the implementation with the existing signing and notarization
> workflow in the private `tractorfm/chatcode` repository where useful.

**Committed changes**

- [`d9a9eef` - Merge Dependabot's actions/checkout 7.0.1 update][commit-d9a9eef]
- [`9604731` - Merge Dependabot's base64 0.23.0 update][commit-9604731]
- [`82ad218` - Configure authenticated Stock API providers][commit-82ad218]
- [`cf5fc4e` - Use the safe base64 implementation][commit-cf5fc4e]
- [`685cdd9` - Sign and notarize macOS release builds][commit-685cdd9]
- [`9eaf576` - Register the macOS signing keychain][commit-9eaf576]
- [`eba5667` - Ignore local Apple signing credentials][commit-eba5667]

**Summary**

The provider choice, Stock API base URL, optional news capability, and bearer
token can now live together in the platform `config.toml`; the environment
token remains the higher-precedence override. The token is validated against
the standard token68 alphabet, sent only to the selected non-redirecting HTTP
adapter, and omitted from printed configuration, debug output, and parser
errors. The endpoint is not fixed: any compatible implementation of the
versioned Stock API contract can be selected by configuration. Provider IDs and
wire-protocol adapters remain compiled Rust implementations behind the
provider-neutral capability traits.

Dependabot's checkout and Base64 updates were reviewed, tested, and merged.
The application disables Base64 0.23's new optional unsafe SIMD default because
OSC 52 clipboard output needs only the scalar standard implementation.

The release workflow isolates Apple credentials in a macOS-only
`macos-release` environment. Both Apple Silicon and Intel binaries receive a
Developer ID Application signature, hardened runtime, and secure timestamp.
Signed disk images must pass integrity checks, return an explicit `Accepted`
notarization result and log without error-level issues, staple successfully,
and pass Gatekeeper before publication. Manual workflow runs exercise the same
path without publishing a release. The secret names match the proven private
Chatcode workflow, while the public project adds mandatory notarization,
stapled disk images, and fail-closed verification.

The first live GitHub-hosted macOS 26 validation confirmed that certificate
import alone is insufficient for `codesign` private-key discovery. The
ephemeral signing keychain is now temporarily added to the user search list,
then the original search list is restored during cleanup. Local PKCS#12
certificate exports and standard App Store Connect private-key filenames are
also ignored defensively so credential setup cannot accidentally stage them.

## 35. Prefer CLI Archives and Explicit Provider Precedence

> Are DMGs really needed for a console application? They do not look like the
> right distribution format. If they are needed only for notarization, omit
> them for now or notarize an archive if Apple supports that.
>
> The `provider` value in `config.toml` should remain effective even when
> Alpaca credentials exist in the user's home configuration. Also add a TOML
> comment explaining where those credentials are stored.

**Committed changes**

- [`8660af2` - Prefer configured providers and macOS archives][commit-8660af2]
- [`d7e2290` - Verify standalone macOS notarization correctly][commit-d7e2290]

**Summary**

macOS releases now use the conventional `tar.gz` CLI artifact only. The
workflow signs the standalone executable with Developer ID and the hardened
runtime, places it in a temporary ZIP accepted by Apple's notarization service,
and deletes that upload container with the ephemeral keychain material. It
requires accepted submit and log responses, then uses Apple's documented
`codesign --check-notarization` path to require an online `notarized`
assessment. The published tarball is extracted again, and its executable is
byte-compared, signature-verified, and checked against the same requirement
before upload. This retains online notarization without publishing a DMG solely
to carry a stapled ticket; standalone executables and tar or ZIP transports do
not provide offline stapling.

Live GitHub-hosted validation exposed that `spctl --type execute` applies
app-oriented policy and can reject a valid bare Mach-O because it is not an app
bundle. After switching to Apple's "other code" verification command, both
Apple Silicon and Intel submissions returned `Accepted` with zero issues and
zero errors, and the final extracted archives satisfied the online notarization
requirement.

Provider choice is now resolved explicitly before any managed credential
lookup: CLI, then environment or `.env`, then platform `config.toml`, then the
Alpaca default. A stored `<config_dir>/credentials.env` pair therefore cannot
select Alpaca or override `provider = "stock-api"`. The example configuration
names both platform paths, the repository-root `config.toml` is ignored as
inactive local secret-bearing state, and malformed typed environment values
fail by variable name instead of silently falling through or exposing their
contents.

## 36. Honor the Configured Provider During Startup

> I configured `provider = "stock-api"` in `config.toml`, but a normal
> `cargo run --release` still says that it is checking Alpaca credentials.
> Make the selected provider effective during startup.

**Committed changes**

- [`976c207` - Prepare stock-tui 0.2.0][commit-976c207]

**Summary**

Startup now resolves the provider before credential handling and reports
provider-neutral initialization progress. An explicit Stock API choice skips
Alpaca credential loading, validation, and onboarding even when Alpaca values
exist in `.env` or the user's configuration directory. CLI and environment
provider overrides retain their documented precedence over platform TOML.

## 37. Refine Status, Help, and Heatmap Controls

> Before the next release, improve the UI and UX:
>
> - Show detailed startup and synchronization progress with numeric counts.
> - Left-align the Help and Data Status columns.
> - Display the application version below the keyboard hints.
> - Underline only a stale ticker symbol, not its entire cell contents.
> - Vertically center Sector cells and show one selectable metric beneath each
>   ticker. Let `i` cycle price, relative gain, absolute gain, sector-relative
>   gain, market cap, and volume.
> - Let `o` reverse the current ordering and `v` switch between the normal grid
>   and StockTouch's clockwise center-out spiral layout.
> - Draw thin frames around favorite cells when space permits.

**Committed changes**

- [`976c207` - Prepare stock-tui 0.2.0][commit-976c207]

**Summary**

The status chrome now exposes bounded phase counts and percentages, overlays
use scan-friendly left-aligned columns, and the action rail includes the
package version. Staleness styling is scoped to the ticker span.

Sector and Starred cells use stable equal dimensions, vertically center a
ticker plus one width-aware metric, preserve signs and units at the minimum
supported viewport, and frame favorites without changing layout. `i` cycles
the six requested metrics. `o` reverses each loaded group in memory while
preserving selection, and `v` maps ranks through a tested clockwise spiral
shared by rendering, keyboard navigation, and mouse hit targets. Detail
headers and chart endpoints also resolve price, volume, and freshness from one
coherent cached observation.

## 38. Expose Overview Controls and Add a Volume Palette

> The `o` control works on the main screen but is not shown as a hint. The `v`
> control is neither active nor shown there; make both available.
>
> The original StockTouch used a different palette when sorting by volume.
> Consider coloring high-to-low volume by brightness while giving each sector
> its own hue.

**Committed changes**

- [`976c207` - Prepare stock-tui 0.2.0][commit-976c207]

**Summary**

Overview now exposes clickable and keyboard-driven Order and View controls,
and grid/spiral presentation is consistent at every heatmap level. Volume
ordering uses log-percentile normalization within each sector to reduce
outlier domination, distinct sector hues in color mode, and brightness for
relative activity. Missing volume remains neutral, focus colors remain
legible, and monochrome terminals retain the same intensity ordering without
hue.

## 39. Consolidate Credentials, Add Range Zoom, and Release 0.2.0

> Allow Alpaca keys in `config.toml` instead of requiring a separate
> `credentials.env`; splitting these settings has no clear benefit.
>
> Add `=`/`+` and `-` shortcuts that zoom in to the next shorter date range and
> out to the next longer range. Address both changes and build a new release.

**Committed changes**

- [`976c207` - Prepare stock-tui 0.2.0][commit-976c207]
- [`v0.2.0` release][release-v0.2.0]

**Summary**

`[providers.alpaca]` now accepts `api_key` and `api_secret`, and successful
onboarding writes that pair into the platform `config.toml` while preserving
comments and unrelated settings. Complete process or `.env` values remain the
highest-precedence pair. The former `credentials.env` location is read only as
an upgrade fallback, so existing users do not need to re-enter credentials.
Secret values stay redacted from errors and debug output, and onboarding makes
the TOML owner-only on Unix.

`=` and `+` zoom toward the next shorter range, while `-` zooms toward the next
longer range across Overview, Sector, Starred, and Detail routes. Search and
modal overlays retain ownership of their input. The `v0.2.0` release packages
five locked cross-platform builds, with signed and notarized Intel and Apple
Silicon macOS executables, plus checksums; publication filters out the
build-only catalog artifact and requires the exact archive count.

## 40. Refresh the Documentation and Screenshots

> Update the README and supporting documentation, and replace its screenshots
> with current captures of the Overview, Sector, and Ticker Detail interfaces.

**Committed changes**

- [`b83cda0` - Refresh v0.2.0 documentation and screenshots][commit-b83cda0]

**Summary**

The README now presents consistent 140x42 captures from the deterministic
v0.2.0 demo, showing the complete nine-sector wall, equal centered Technology
tiles with a starred ticker and single selected metric, and the one-year NVDA
detail chart with its cursor, axes, volume, statistics, news, rank, and version
chrome. The accompanying prose documents overlay-owned controls, numeric
synchronization progress, ticker-only stale underlining, cached-company search,
sort-direction defaults, and the private development provider boundary.

Supporting architecture, cache, provider, and HTTP-contract documents now use
provider-neutral lifecycle terminology where appropriate and name the actual
`stock-api` news settings. The Unreleased changelog records the post-v0.2.0
documentation refresh.

## 41. Make Volume Mode Range-Aware

> The special Volume colorization currently appears to use the latest daily
> volume for every date range, so switching ranges does not meaningfully change
> it. Limit that behavior to `1D`, or preferably calculate the proper volume for
> every selected range so Volume mode responds to range changes as well.

**Committed changes**

- [`d8c596a` - Make heatmap volume range-aware][commit-d8c596a]

**Summary**

Volume ordering, sector tile values, and sector-relative brightness now use
cumulative share volume from the selected period. `1D` keeps the selected
snapshot's latest-session cumulative volume, with an inclusive cached-bar
fallback; longer ranges sum one canonical cached OHLCV timeframe without
mixing granularities or adding the daily snapshot twice. Daily bars are
preferred through `2Y`, weekly bars for `5Y` through `ALL`, and missing
range history remains neutral and sorts after known totals.

The aggregate uses the existing indexed `(symbol, timeframe, timestamp)` cache
path and bypasses SQL entirely for the normal `1D` snapshot case. Regression
coverage verifies range-dependent values and ordering, daily-versus-weekly
selection, inclusive cutoff handling, snapshot fallback, and rejection of a
weekly bar as a one-day substitute. The README, changelog, and data/cache
architecture documents now describe the same semantics.

## 42. Release 0.2.1 and Assess Package Managers

> Create a new release. Also assess how easily `stock-tui` could be distributed
> through popular package managers such as Homebrew and APT.

**Committed changes**

- [`92340b8` - Prepare stock-tui 0.2.1][commit-92340b8]
- [`v0.2.1` release][release-v0.2.1]

**Summary**

The `v0.2.1` patch release rolls the range-aware cumulative Volume behavior
and post-`v0.2.0` documentation refresh into five locked platform archives.
The existing release pipeline signs and notarizes both macOS executables,
publishes static Linux builds and the Windows archive, and attaches one
checksum manifest after validating the exact artifact set.

Package-manager review identified crates.io and a project-controlled Homebrew
tap as the lowest-effort next channels. The crate already passes Cargo's
publish dry run and fits comfortably under its package-size limit; a tap can
consume immutable release sources and provide one-command installs without an
external acceptance gate. Standalone `.deb` assets are similarly
straightforward, while a signed APT repository or Ubuntu PPA adds repository,
key, source-package, and ongoing update responsibilities. Homebrew Core and
Debian's official archive are better revisited after broader adoption because
their acceptance, policy, and sponsorship processes are intentionally more
demanding.

## 43. Diagnose INHD and Improve Sparse-History Detail UX

> Check whether the INHD chart in the supplied macOS screenshot is rendered
> correctly; its shape looks suspicious.
>
> The app was first launched with Alpaca credentials and then with `stock-api`,
> but mixed providers probably are not the cause. Address the issue if it is
> real, and also:
>
> 1. Keep every range selectable, but mute fixed ranges such as `5Y` and `10Y`
>    when the ticker has less observed history. Show the complete available
>    span and its first date.
> 2. When an Overview benchmark has focus, remove the simultaneous sector
>    highlight so there is only one visual selection.
> 3. When space permits, show an exact price beside the chart-cursor
>    intersection and a suitable date or month beside the X-axis intersection.
>    Place both labels on the roomier side of the cursor.

**Committed changes**

- [`f983874` - Fix halted ticker charts and history UX][commit-f983874]

**Summary**

The investigation reproduced the INHD shape from real cached data. Its June
price jump is a genuine observation, but post-halt flat, zero-volume,
zero-trade provider rows had incorrectly extended it into a fresh plateau.
Those rows remain available in raw SQLite while price endpoints, freshness,
timeframe choice, cached-history coverage, and detail charts now use traded
observations only. Price traces, fills, volume, axes, and cursor positions map
to actual timestamps, leaving long gaps and a halted tail blank; keyboard
navigation skips blank columns. The lower price bound is clamped to zero, and
bounded cursor labels show the selected price and range-aware time.

Ticker detail now reports its cached history span and boundary dates, while
longer fixed ranges are muted without losing mouse targets or shortcuts.
Overview benchmark and sector focus are visually exclusive. The same work also
corrected INHD's inflated market cap: provider snapshot caps take precedence,
and Alpaca-backed local estimates adjust dated SEC share counts through cached
forward and reverse split history before multiplying by price. Failed or
unsupported split coverage fails closed instead of combining stale shares with
a post-split price. The behavior is documented and covered by storage,
provider, synchronization, chart, interaction, and responsive rendering tests.

## 44. Drain Mouse Input Before Restoring the Shell

> Address [GitHub issue #10][issue-10]: quitting while mouse-motion reports are
> still in flight must not leave SGR escape sequences or coordinate fragments
> for the next shell prompt to display or execute.

**Committed changes**

- [`98f408a` - Drain terminal input before shell restore][commit-98f408a]

**Summary**

Terminal shutdown now has two explicit phases. The app disables bracketed
paste, focus events, and SGR mouse reporting and flushes that output while the
terminal is still raw; it then drops the event reader, cancels and joins cache
workers, drains Crossterm events through a 40 ms quiet period with a 200 ms
hard cap, flushes the OS input queue, leaves the alternate screen, shows the
cursor, and restores canonical input and echo last. Cleanup attempts every
operation while preserving the first error, with an immediate best-effort path
for setup failures, `Drop`, and panics.

Demo generation and live history writes now observe cooperative cancellation,
and partially written bar batches roll back transactionally. PTY-backed tests
inject both immediate and delayed SGR motion reports, verify cleanup ordering
and bounded exit latency, simulate the next shell reader, and cover normal,
early-demo, and panic exits with canonical mode and echo restored.

## 45. Release 0.2.2

> Build and publish a new release containing the completed chart, market-cap,
> history UX, and terminal-shutdown work.

**Committed changes**

- [`ab6b3ec` - Prepare stock-tui 0.2.2][commit-ab6b3ec]
- [`v0.2.2` release][release-v0.2.2]

**Summary**

The `v0.2.2` patch release packages timestamp-aware sparse charts, cached
history coverage and cursor labels, exclusive benchmark focus, split-adjusted
market-cap estimates, and the two-phase terminal shutdown that prevents
in-flight SGR mouse reports from reaching the next shell prompt.

Release preparation also makes manual workflow dispatches unconditionally
build-only, even when a tag is selected, while reserving publication for a
`v*` tag push. The maintainer checklist now requires a complete build-only
preflight from `main` and post-publication archive, checksum, notarization, and
release-note verification.

## 46. Update and Verify Signed Release Binaries

> Explain how to update `stock-tui` on this Linux machine and on macOS, and how
> to verify that the downloaded macOS binary is signed and notarized correctly.

**Relevant committed release**

- [`ab6b3ec` - Prepare stock-tui 0.2.2][commit-ab6b3ec]
- [`v0.2.2` release][release-v0.2.2]

**Summary**

The update procedure uses the release archive matching the operating system and
CPU, verifies it against `SHA256SUMS`, and replaces the existing executable.
On macOS, `codesign --verify --deep --strict --verbose=2`,
`codesign -dv --verbose=4`, and `spctl --assess --type execute --verbose=4`
verify the Developer ID signature and Gatekeeper notarization assessment. This
was operational guidance for the already published artifacts and required no
new source change.

## 47. Compress Closed-Market Time Instead of Drawing Chart Gaps

> The new no-trade gaps make charts harder to read. Model `1D` as the latest
> observed exchange session from its regular open through close, leaving only
> the not-yet-observed portion of an active session blank. For longer ranges,
> omit nights, weekends, and holidays as StockTouch did, so one week presents
> five observed trading sessions without empty calendar gaps.

**Committed changes**

- [`5f85037` - Harden market sessions and SEC share coverage][commit-5f85037]

**Summary**

Detail charts now construct range-specific timelines. `1D` selects the newest
observed exchange-local session and maps its full regular trading window.
`1W` concatenates the five newest observed sessions; longer intraday histories
compress closed-market time, while daily and weekly histories use ordinal
observation spacing. Real intraday gaps carry the last trade forward without
fabricating volume, and axes, fills, cursor positions, and volume columns share
the same timeline.

## 48. Keep Provider Caches Bound to One Market Context

> Since providers may eventually cover different exchanges, do not assume every
> market uses New York hours. Keep data from incompatible providers, feeds,
> exchanges, calendars, symbol namespaces, or currencies from mixing. Use one
> coherent market context per launch, or clear unrelated cached observations
> before rendering them.

**Committed changes**

- [`5f85037` - Harden market sessions and SEC share coverage][commit-5f85037]

**Summary**

Providers now expose an opaque dataset namespace plus a normalized market
context containing its calendar, symbol namespace, currency, IANA timezone,
and regular session bounds. SQLite schema 3 stamps that identity and clears
incompatible market observations, memberships, news, and sync checkpoints
transactionally while retaining favorites for rehydration. Alpaca feed and
endpoint changes and `stock-api` endpoint or schema changes therefore cannot
silently reuse unattributable rows.

## 49. Preserve GOOG as Alphabet's Compact Canonical Ticker

> GOOGL has appeared again even though the compact heatmap should prefer the
> four-character GOOG ticker. Keep GOOG as Alphabet's canonical sector member.

**Committed changes**

- [`5f85037` - Harden market sessions and SEC share coverage][commit-5f85037]

**Summary**

Catalog reconciliation explicitly retires a former GOOGL sector membership
when the current catalog selects GOOG. The two securities remain independent
for prices, bars, search, and favorites; this is a membership migration, not an
unsafe ticker alias.

## 50. Fill Cursor Columns, Refine Range Hints, and Restore Dell Market Cap

> Fix three observations:
>
> 1. Every interior chart column should produce a cursor value. When no distinct
>    observation maps to a terminal cell, repeat the preceding value.
> 2. Do not mute `10Y` merely because a ticker has less than ten complete years;
>    keep it normal whenever it exposes history beyond `5Y`.
> 3. Investigate and restore the missing DELL market-cap value.

**Committed changes**

- [`5f85037` - Harden market sessions and SEC share coverage][commit-5f85037]

**Summary**

Hover selection now repeats the preceding observation only across blank
interior columns and leaves pre-history and future-session tails empty. Range
styling compares the effective cached interval with the next-shorter preset, so
partial ten-year history remains visibly useful. The catalog builder resolves
DELL from the latest filing's reviewed equal-economic Class A, B, and C
aggregate and retains its exact fact provenance.

## 51. Resolve the Remaining Ambiguous Market-Cap Share Bases

> Several notable companies still show no market cap, including STZ, HSY, TSN,
> BF.A, COKE, CMCSA, FWONA, DKNG, WSO, SUN, BSM, KRP, WTTR, METC, SPG, BAM,
> UPS, LEN, and HVII. Find any other affected top companies and implement a
> general, reviewable solution that also fails safely when future filings
> change.

**Committed changes**

- [`5f85037` - Harden market sessions and SEC share coverage][commit-5f85037]

**Summary**

The SEC builder now resolves each ambiguous issuer from its latest inline
filing and exact XBRL class dimensions. A declarative policy registry records
reviewed member multipliers, accession-scoped economic facts, price basis,
confidence, rationale, and official filing URLs for multi-class, tracking
stock, Up-C, partnership, separately traded class, and SPAC structures.
Unknown or changed accessions, namespaces, units, periods, dimensions,
members, or duplicate facts fail closed.

The audited catalog resolves all 1,880 candidates and all 900 sector top-100
positions, including the additional LBRDA gap found by the audit. Publication
and release workflows reject any future unresolved top-100 row. Corrected
filing-specific calculations for ERIE, PJT, MC, HLNE, and Visa are covered by
fixtures and adversarial policy-drift tests.

## 52. Publish a Fresh Catalog and Release 0.2.3

> Push the completed work, build and publish a fresh catalog if the action is
> required, and create a new release.

**Committed changes**

- [`5f85037` - Harden market sessions and SEC share coverage][commit-5f85037]
- [`4fa796a` - Prepare stock-tui 0.2.3][commit-4fa796a]
- [`v0.2.3` release][release-v0.2.3]

**Summary**

The catalog action must run before the release tag because release builds
download and embed the stable hosted catalog rather than trusting the
repository fallback. The release sequence therefore pushes `main`, publishes
and verifies the fresh checksummed R2 catalog, runs the five-platform build-only
preflight, and only then pushes `v0.2.3`. The tag-triggered workflow publishes
the five platform archives and checksum manifest after verifying the signed and
notarized Intel and Apple Silicon macOS executables.

## 53. Install 0.2.3 and Experiment with Volume Trails

> Explain how to download and install the latest release on Linux and macOS.
>
> Also try a local chart experiment: when sparse volume observations leave
> horizontal gaps, fill those gaps with dimmed bars that repeat the preceding
> height. The result should read as a visual trail or shadow beside the real
> volume bar.

**Committed changes**

- [`6764296` - Add dim volume chart trails][commit-6764296]

**Summary**

The installation guidance covered the matching release archive, checksum
verification, extraction, and executable placement for Linux and macOS. The
chart experiment keeps observed volume bars solid while filling unoccupied
columns after the first positive observation with a subdued same-height trail.
The trail is presentation-only: it does not create cached observations or
change aggregation, statistics, or chart scaling.

## 54. Extend the Volume Trail Through the Chart Tail

> The experiment looks good. When there is room after the final observed
> volume bar, continue the same dimmed trail through the end of the chart.

**Committed changes**

- [`6764296` - Add dim volume chart trails][commit-6764296]

**Summary**

The renderer now carries the last observed height through every remaining
unoccupied chart column. Leading columns before the first positive bar remain
empty, real observations retain their stronger color, and cursor emphasis does
not make synthetic trail columns look like measured volume.

## 55. Release 0.2.4

> Build and ship a new release with the accepted volume-trail rendering.

**Committed changes**

- [`6764296` - Add dim volume chart trails][commit-6764296]
- [`042d825` - Prepare stock-tui 0.2.4][commit-042d825]
- [`v0.2.4` release][release-v0.2.4]

**Summary**

Version `0.2.4` packages the volume-trail refinement for all supported
platforms. The release process validates the Rust workspace, runs the
five-platform build-only preflight, verifies signing and notarization for both
macOS architectures, and publishes platform archives with a shared checksum
manifest from the annotated release tag.

## 56. Bound Ongoing Volume Trails and Add Company Context

> For an ongoing chart, stop the dim volume trail at the end of the upper price
> trace instead of filling the future session tail.
>
> Also show a brief company description in detailed ticker views when the
> terminal has enough room.

**Committed changes**

- [`c1d5ede` - Refine ongoing charts and company context][commit-c1d5ede]

**Summary**

Synthetic volume columns now stop at the last rendered price column, while
completed ranges still reach their natural endpoint. The roomy detail layout
uses its existing company panel for concise issuer context without reducing
compact-chart space. Fresh catalogs join issuer SIC codes to official SEC
taxonomy labels through one versioned taxonomy document; older schema-v2
catalogs remain valid and use a clean exchange, SIC, and sector fallback.
Catalog and release workflows reject unsafe or missing labels in newly
published artifacts.

## 57. Publish the Catalog and Release 0.2.5

> Push the completed changes, publish the enriched catalog, and ship a new
> release.

**Committed changes**

- [`c1d5ede` - Refine ongoing charts and company context][commit-c1d5ede]
- [`07cdf18` - Prepare stock-tui 0.2.5][commit-07cdf18]
- [`v0.2.5` release][release-v0.2.5]

**Summary**

The release sequence pushes the reviewed implementation and version metadata,
publishes and verifies the enriched SEC catalog, and then runs the five-platform
build-only preflight. The annotated `v0.2.5` tag publishes Linux, Windows, and
signed/notarized Intel and Apple Silicon macOS archives plus a shared checksum
manifest only after every release job succeeds.

## 58. Replace Classification Copy with Useful Company Context

> Is "listed on this exchange and classified by the SEC in this industry" the
> most complete company description available? If it is, make the wording more
> human-friendly and precise. If the SEC does not provide a real introduction,
> investigate a better source.

**Committed changes**

- [`d7d0dea` - Enrich durable company profiles][commit-d7d0dea]

**Summary**

The catalog now enriches issuers with concise CC0 Wikidata descriptions and
industry facts matched strictly by SEC CIK. The client prefers that context,
then presents the exchange, symbol, and SIC industry as separate facts; missing
or ambiguous mappings use a readable classification fallback. Input
validation, source provenance, coverage gates, responsive detail layout, and
documentation keep the enrichment safe and explicit. The longer-term
issuer-authored source remains the business section of the latest annual SEC
filing.

## 59. Cache Slow-Changing Company Introductions

> Store introductions for every ticker until the next intentional update.
> Company descriptions should rarely change substantially, so full enrichment
> can be a heavier task that runs much less often than the normal catalog
> update.

**Committed changes**

- [`d7d0dea` - Enrich durable company profiles][commit-d7d0dea]

**Summary**

Catalog generation now keeps a bounded, versioned per-CIK snapshot of both
successful and empty profile lookups. Daily publications query only new,
materially renamed, or algorithm-stale issuers; the monthly or manually
requested full pass bypasses query caches while retaining last-known-good
profiles during transient omissions. Stable and content-addressed R2 objects
preserve the builder snapshot independently of GitHub Actions cache retention,
while runtime clients continue to receive descriptions only through the compact
catalog.

## 60. Publish the Enriched Catalog

> Push the completed company-profile changes and run the catalog publication.

**Committed changes**

- [`d7d0dea` - Enrich durable company profiles][commit-d7d0dea]
- [`9d04968` - Document company profile prompts][commit-9d04968]
- [Successful catalog publication][catalog-run-30578835793]

**Summary**

The reviewed implementation and prompt history were pushed to `main`, then the
catalog workflow ran with a full profile refresh. It published and independently
verified 1,877 catalog companies, 985 enriched descriptions, 597 enriched
top-900 companies, and the durable positive/negative R2 profile snapshot. The
published catalog payload also matched its manifest size and SHA-256 digest.

## 61. Restore Market Cap for Share-Class Tickers

> `BF.A` and `BRK.B` appear to be missing market-cap values.

**Committed changes**

- [`31b1b2c` - Normalize SEC share-class symbols][private-commit-31b1b2c]
- [`0f243ab` - Preserve catalog provenance in stock API][private-commit-0f243ab]
- [Successful private worker deployment][stock-api-deploy-30585050528]

**Summary**

The SEC catalog already contained reviewed share estimates for both issuers,
but the private worker kept its hyphenated catalog symbols while provider
snapshots used dotted symbols. Catalog ingestion now converts share-class and
preferred-share suffixes to the API's runtime notation before validation,
lookup, and D1 persistence. End-to-end tests cover `BF-A` to `BF.A`, `BRK-B` to
`BRK.B`, preferred-series notation, market-cap enrichment, cache reuse, and
post-normalization collision rejection. Compact nested fact provenance is also
retained, and deployment smoke tests now require a fresh response followed by
an identical cache hit with complete capitalization provenance for both
share-class symbols.

## 62. Deploy the Fix and Release v0.2.6

> Deploy the private worker changes and create a new stock-tui release.

**Committed changes**

- [`0f243ab` - Preserve catalog provenance in stock API][private-commit-0f243ab]
- [`f68cb94` - Prepare stock-tui 0.2.6][commit-f68cb94]
- [Successful private worker deployment][stock-api-deploy-30585050528]
- [`v0.2.6` release][release-v0.2.6]

**Summary**

The private worker was deployed with deterministic production checks for
`NVDA`, `BF.A`, and `BRK.B`, including current prices, reviewed share counts,
estimated market caps, provenance fields, and byte-identical R2 cache reuse.
The client release packages company-specific CC0 descriptions and durable
catalog-profile updates, documents the canonical dotted share-class contract,
and ships five checksummed platform archives with signed and notarized macOS
executables.

## 63. Replace Boilerplate Company Introductions

> I still do not see any meaningful company introductions in the ticker-detail
> view.

**Committed changes**

- [`d0f1a20` - Improve company profile matching][commit-d0f1a20]
- [`cbe2cea` - Apply catalog updates atomically][commit-cbe2cea]
- [`7bf25d0` - Prepare stock-tui 0.2.7][commit-7bf25d0]
- [Successful full catalog publication][catalog-run-30590280654]
- [`v0.2.7` release][release-v0.2.7]

**Summary**

The screenshot exposed both an old running `v0.2.5` process and a weak Amazon
legal-entity profile. The catalog builder now rejects legal, promotional,
location-only, and stale-status boilerplate, then resolves a canonical
Wikidata company only with a unique normalized name, business hierarchy, and
matching active ticker/exchange evidence. It fails closed on ambiguity,
deprecated or ended listings, dissolved entities, subsidiaries, truncated
results, and known homonym collisions. Structured industry and fallback
product/service facts provide concise business context when safe.

The durable profile store and publication gates were versioned together. A
full refresh published 1,082 profiles across 1,877 companies, including 640 of
the 900 visible sector members, and canonical AMZN, WMT, and CSCO canaries.
Runtime catalog installation and membership replacement are atomic and
off-thread; in-flight provider requests cannot restore stale descriptions or
derive market caps from mismatched share metadata. The release ships five
checksummed archives, including signed and notarized macOS executables.

## 64. Remove Redundant History and Repair News Rows

> Remove the `HISTORY` item from ticker Statistics because the same information
> is already shown in the detail header and looks out of place in that panel.
>
> Polish the related-news section shown in the supplied screenshot:
>
> 1. Ensure every visible article includes its date and source, including long
>    selected headlines.
> 2. Remove the unexplained blank line that sometimes appears after an
>    article's date/source and before the next headline.

**Committed changes**

- [`d95e9c8` - Polish ticker news layout][commit-d95e9c8]

**Summary**

Ticker Statistics no longer repeats the cached-history span already present in
the header, and the reclaimed row expands the News panel. News items now size
themselves to one to three wrapped headline lines plus a dedicated metadata
line, so short items have no spacer and long items cannot clip their date or
source. A fixed selection gutter prevents hover-induced wrapping changes, and
stable pages keep keyboard-selected articles visible without moving rows under
the mouse. Responsive buffer tests cover the original long-headline failure,
adjacent rows, metadata fallback, minimum `60x20` rendering, late selection,
and hover stability.

## 65. Release v0.3.0

> Create and publish a new `v0.3.0` release.

**Committed changes**

- [`d95e9c8` - Polish ticker news layout][commit-d95e9c8]
- [`10b564d` - Prepare stock-tui 0.3.0][commit-10b564d]
- [Successful `main` CI][ci-run-30629067495]
- [Successful five-platform build-only preflight][release-preflight-run-30629238846]
- [Successful tagged release workflow][release-run-30629889605]
- [`v0.3.0` release][release-v0.3.0]

**Summary**

Version `0.3.0` packages the streamlined ticker Statistics panel and adaptive
related-news rows that keep complete date/source metadata attached, preserve
stable mouse positions, and scroll keyboard focus into view. Cross-platform
`main` CI and a non-publishing five-platform preflight validated the release
commit before the annotated tag published checksummed Linux, Windows, and
signed and notarized Intel and Apple Silicon macOS archives.

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
[commit-0b6e198]: https://github.com/chatcode-lab/stock-tui/commit/0b6e1980466415a54e0c64e4395fe7d0684db2b3
[commit-ce13883]: https://github.com/chatcode-lab/stock-tui/commit/ce13883ecfe36219679e398385dcb0a905002431
[commit-d9a9eef]: https://github.com/chatcode-lab/stock-tui/commit/d9a9eefd69120181a85af31e5b27190acc7673e3
[commit-9604731]: https://github.com/chatcode-lab/stock-tui/commit/9604731bcc01bdf44131cd1601632858e71ee65f
[commit-82ad218]: https://github.com/chatcode-lab/stock-tui/commit/82ad218df44c445db35832434c04e58d5613e1f4
[commit-cf5fc4e]: https://github.com/chatcode-lab/stock-tui/commit/cf5fc4e23e04d6ab60e6de70729076c423de44a9
[commit-685cdd9]: https://github.com/chatcode-lab/stock-tui/commit/685cdd9f4dd8d62ce43dbfa66a94d5d4f141b34f
[commit-9eaf576]: https://github.com/chatcode-lab/stock-tui/commit/9eaf57695ef6061c4e3c28a9b9f072baa9441d4f
[commit-eba5667]: https://github.com/chatcode-lab/stock-tui/commit/eba566713605468353c9e488e0ffeac634d2ebc6
[commit-8660af2]: https://github.com/chatcode-lab/stock-tui/commit/8660af2152dca79a804479e1eea36e94d21a9aa8
[commit-d7e2290]: https://github.com/chatcode-lab/stock-tui/commit/d7e22905ccd0a2236c0779b73c0ecd3330764c81
[commit-976c207]: https://github.com/chatcode-lab/stock-tui/commit/976c207d02ead76ccbfebc2e275ffac3dfdf4999
[commit-b83cda0]: https://github.com/chatcode-lab/stock-tui/commit/b83cda0724af86ed7f504d2efdbba3958427a9cc
[commit-d8c596a]: https://github.com/chatcode-lab/stock-tui/commit/d8c596a3fa3ce6d6f0cb25c6ea506d3abb9cab56
[commit-92340b8]: https://github.com/chatcode-lab/stock-tui/commit/92340b84be0f261cd3a8f5f71432320a679bf583
[commit-f983874]: https://github.com/chatcode-lab/stock-tui/commit/f983874627056e8acf2f7d02d6d1581b3f9b80b6
[commit-98f408a]: https://github.com/chatcode-lab/stock-tui/commit/98f408a72de6b8ed5fb53ac3e4da55fbf1aad3be
[commit-ab6b3ec]: https://github.com/chatcode-lab/stock-tui/commit/ab6b3ecd1204558a6f48c69a3a35dae99d1963ac
[commit-5f85037]: https://github.com/chatcode-lab/stock-tui/commit/5f8503730a376f48f00c689e843334bee41988d7
[commit-4fa796a]: https://github.com/chatcode-lab/stock-tui/commit/4fa796a1660c5d546e571baf2ebc538a9da83745
[commit-6764296]: https://github.com/chatcode-lab/stock-tui/commit/67642963c1a03bb0cf41d549e9a63da04db2001d
[commit-042d825]: https://github.com/chatcode-lab/stock-tui/commit/042d825bdd090298f262992335beedb196589c2c
[commit-c1d5ede]: https://github.com/chatcode-lab/stock-tui/commit/c1d5ede766aea9542fc383a6528b73d470eab60b
[commit-07cdf18]: https://github.com/chatcode-lab/stock-tui/commit/07cdf189d07974baf5ed1fe8b3e13729dce5b036
[commit-d7d0dea]: https://github.com/chatcode-lab/stock-tui/commit/d7d0dea6378cb96aad93ff0538c869ee8f7c44b5
[commit-9d04968]: https://github.com/chatcode-lab/stock-tui/commit/9d049685326e523f80aebed496b26f7d4b5e293e
[commit-f68cb94]: https://github.com/chatcode-lab/stock-tui/commit/f68cb942dee9e47343a7539800c12fdd83422edc
[commit-d0f1a20]: https://github.com/chatcode-lab/stock-tui/commit/d0f1a20bbc60ccd5eb9e62a24010563a808329c0
[commit-cbe2cea]: https://github.com/chatcode-lab/stock-tui/commit/cbe2cea2ae56c6d5f2c09e49c4ff1450f188fa6c
[commit-7bf25d0]: https://github.com/chatcode-lab/stock-tui/commit/7bf25d0c8de670160d36cac7b02e4826a469dcf3
[commit-d95e9c8]: https://github.com/chatcode-lab/stock-tui/commit/d95e9c809d3c363da0c9478530c7e1b47f0b86c4
[commit-10b564d]: https://github.com/chatcode-lab/stock-tui/commit/10b564dbe42ef98675e9d64a23a6bd8274f2f0a9
[catalog-run-30590280654]: https://github.com/chatcode-lab/stock-tui/actions/runs/30590280654
[catalog-run-30578835793]: https://github.com/chatcode-lab/stock-tui/actions/runs/30578835793
[ci-run-30629067495]: https://github.com/chatcode-lab/stock-tui/actions/runs/30629067495
[release-preflight-run-30629238846]: https://github.com/chatcode-lab/stock-tui/actions/runs/30629238846
[release-run-30629889605]: https://github.com/chatcode-lab/stock-tui/actions/runs/30629889605
[private-commit-a67e660]: https://github.com/chatcode-lab/stock-api/commit/a67e660f53e754c8e2bf45ba3b3a1ea8ab5fbd42
[private-commit-59bd27f]: https://github.com/chatcode-lab/stock-api/commit/59bd27f4df6adc258ae1e2c310480f7570b739c1
[private-commit-75e605b]: https://github.com/chatcode-lab/stock-api/commit/75e605bb71780af13826c0355b629ad1a7378ca4
[private-commit-210886a]: https://github.com/chatcode-lab/stock-api/commit/210886afc358d75efec0f1977831cd4ed0d4f6d7
[private-commit-31901f8]: https://github.com/chatcode-lab/stock-api/commit/31901f89d924373f9fa76f09d581c18d8ba4149c
[private-commit-6bf23bb]: https://github.com/chatcode-lab/stock-api/commit/6bf23bb3c765ee871d5ef3e63ea5c8f91d3d6c40
[private-commit-cc1f6ca]: https://github.com/chatcode-lab/stock-api/commit/cc1f6ca2a0aaadc5a76dbd99b942e49f6aa58b1d
[private-commit-31b1b2c]: https://github.com/chatcode-lab/stock-api/commit/31b1b2c1062cd8c27822dc23a76c8e3068592183
[private-commit-0f243ab]: https://github.com/chatcode-lab/stock-api/commit/0f243abd74a38d0bb94bbf7d5d8720b054085ea3
[stock-api-deploy-30585050528]: https://github.com/chatcode-lab/stock-api/actions/runs/30585050528
[pr-6]: https://github.com/chatcode-lab/stock-tui/pull/6
[issue-10]: https://github.com/chatcode-lab/stock-tui/issues/10
[release-v0.1.0]: https://github.com/chatcode-lab/stock-tui/releases/tag/v0.1.0
[release-v0.1.1]: https://github.com/chatcode-lab/stock-tui/releases/tag/v0.1.1
[release-v0.2.0]: https://github.com/chatcode-lab/stock-tui/releases/tag/v0.2.0
[release-v0.2.1]: https://github.com/chatcode-lab/stock-tui/releases/tag/v0.2.1
[release-v0.2.2]: https://github.com/chatcode-lab/stock-tui/releases/tag/v0.2.2
[release-v0.2.3]: https://github.com/chatcode-lab/stock-tui/releases/tag/v0.2.3
[release-v0.2.4]: https://github.com/chatcode-lab/stock-tui/releases/tag/v0.2.4
[release-v0.2.5]: https://github.com/chatcode-lab/stock-tui/releases/tag/v0.2.5
[release-v0.2.6]: https://github.com/chatcode-lab/stock-tui/releases/tag/v0.2.6
[release-v0.2.7]: https://github.com/chatcode-lab/stock-tui/releases/tag/v0.2.7
[release-v0.3.0]: https://github.com/chatcode-lab/stock-tui/releases/tag/v0.3.0
