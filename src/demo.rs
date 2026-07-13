//! Deterministic offline market data used when Alpaca credentials are absent.

use chrono::{DateTime, Datelike, Days, Duration, NaiveDate, NaiveTime, Utc, Weekday};

use crate::domain::{Bar, Company, NewsItem, Sector, Snapshot};

pub const COMPANIES_PER_SECTOR: usize = 100;

const FIVE_MINUTE_BARS: usize = 78;
const HOURLY_SESSIONS: usize = 24;
const HOURS_PER_SESSION: usize = 7;
const DAILY_BARS: usize = 264;
const WEEKLY_BARS: usize = 263;

/// A complete demo-market payload ready for bulk insertion into storage.
#[derive(Debug, Clone)]
pub struct DemoDataset {
    pub companies: Vec<Company>,
    pub bars: Vec<Bar>,
    pub snapshots: Vec<Snapshot>,
    pub news: Vec<NewsItem>,
}

/// Builds a deterministic market whose dates track the supplied clock.
///
/// Passing the same `as_of` value always produces byte-for-byte equivalent
/// scalar values and ordering. Prices and headlines are simulated and must not
/// be treated as investment information.
#[must_use]
pub fn generate(as_of: DateTime<Utc>) -> DemoDataset {
    let company_count = Sector::ALL.len() * COMPANIES_PER_SECTOR;
    let bars_per_company =
        FIVE_MINUTE_BARS + HOURLY_SESSIONS * HOURS_PER_SESSION + DAILY_BARS + WEEKLY_BARS;
    let anchor = last_completed_market_close(as_of);
    let mut dataset = DemoDataset {
        companies: Vec::with_capacity(company_count),
        bars: Vec::with_capacity(company_count * bars_per_company),
        snapshots: Vec::with_capacity(company_count),
        news: Vec::with_capacity(company_count * 2),
    };

    for (sector_index, sector) in Sector::ALL.into_iter().enumerate() {
        for rank in 1_u16..=u16::try_from(COMPANIES_PER_SECTOR).unwrap_or(100) {
            let (company, model) = make_company(sector, sector_index, rank, as_of);
            dataset.snapshots.push(make_snapshot(&model, as_of));
            append_bars(&mut dataset.bars, &model, anchor);
            dataset
                .news
                .extend(make_news(&company, sector_index, rank, as_of));
            dataset.companies.push(company);
        }
    }

    dataset
}

#[derive(Debug, Clone)]
struct PriceModel {
    symbol: String,
    seed: u64,
    current_price: f64,
    previous_close: f64,
    annual_drift: f64,
    cycle_phase: f64,
    daily_volume: f64,
    daily_trades: u64,
}

impl PriceModel {
    fn price_days_ago(&self, days_ago: f64) -> f64 {
        let days_ago = days_ago.max(0.0);
        let log_price = if days_ago <= 1.0 {
            let progress = 1.0 - days_ago;
            let session_wave =
                (progress * std::f64::consts::PI).sin() * (unit(self.seed, 41) - 0.5) * 0.018;
            self.previous_close.ln()
                + progress * (self.current_price / self.previous_close).ln()
                + session_wave
        } else {
            let elapsed = days_ago - 1.0;
            let long_cycle = 0.11
                * ((self.cycle_phase + days_ago / 67.0).sin()
                    - (self.cycle_phase + 1.0 / 67.0).sin());
            let short_cycle = 0.045
                * ((self.cycle_phase * 1.7 + days_ago / 13.0).sin()
                    - (self.cycle_phase * 1.7 + 1.0 / 13.0).sin());
            self.previous_close.ln() - self.annual_drift * elapsed / 365.25
                + long_cycle
                + short_cycle
        };
        log_price.exp().max(0.5)
    }
}

fn make_company(
    sector: Sector,
    sector_index: usize,
    rank: u16,
    as_of: DateTime<Utc>,
) -> (Company, PriceModel) {
    let featured = featured_companies(sector);
    let rank_index = usize::from(rank - 1);
    let (symbol, name) = featured.get(rank_index).map_or_else(
        || generated_identity(sector, rank),
        |(symbol, name)| ((*symbol).to_owned(), (*name).to_owned()),
    );
    let seed = hash64(&symbol) ^ (u64::from(rank) << 32);
    let sector_base = sector_market_cap(sector);
    let market_cap = sector_base / f64::from(rank).powf(0.94);
    let current_price = 14.0 + unit(seed, 1) * 486.0;
    let return_bucket =
        (u32::from(rank) * 37 + u32::try_from(sector_index).unwrap_or_default() * 23) % 101;
    let day_return = (f64::from(return_bucket) - 50.0) / 650.0;
    let previous_close = current_price / (1.0 + day_return);
    let annual_drift = -0.10 + unit(seed, 2) * 0.42;
    let turnover = 0.0025 + unit(seed, 3) * 0.015;
    let daily_volume = market_cap / current_price * turnover;
    let daily_trades = 12_000 + mixed(seed, 4) % 1_800_000;
    let industry = industries(sector)[rank_index % industries(sector).len()].to_owned();
    let company = Company {
        symbol: symbol.clone(),
        name: name.clone(),
        sector: Some(sector),
        raw_sector: Some(sector.label().to_owned()),
        exchange: if (rank + u16::try_from(sector_index).unwrap_or_default()).is_multiple_of(2) {
            "NASDAQ".to_owned()
        } else {
            "NYSE".to_owned()
        },
        industry: industry.clone(),
        market_cap: Some(market_cap),
        shares_outstanding: Some(market_cap / current_price),
        rank: Some(rank),
        description: format!(
            "{name} represents the {industry} industry in the offline {sector} demo universe. All displayed values are simulated."
        ),
        in_universe: true,
        retained: false,
        updated_at: as_of,
    };
    let model = PriceModel {
        symbol,
        seed,
        current_price,
        previous_close,
        annual_drift,
        cycle_phase: unit(seed, 5) * std::f64::consts::TAU,
        daily_volume,
        daily_trades,
    };
    (company, model)
}

fn make_snapshot(model: &PriceModel, as_of: DateTime<Utc>) -> Snapshot {
    let open = model.price_days_ago(0.27);
    let wick = 0.004 + unit(model.seed, 6) * 0.012;
    Snapshot {
        symbol: model.symbol.clone(),
        price: Some(model.current_price),
        previous_close: Some(model.previous_close),
        open: Some(open),
        high: Some(open.max(model.current_price) * (1.0 + wick)),
        low: Some(open.min(model.current_price) * (1.0 - wick)),
        volume: Some(model.daily_volume * (0.82 + unit(model.seed, 7) * 0.36)),
        updated_at: as_of,
    }
}

fn append_bars(bars: &mut Vec<Bar>, model: &PriceModel, anchor: DateTime<Utc>) {
    append_weekly_bars(bars, model, anchor);
    append_daily_bars(bars, model, anchor);
    append_hourly_bars(bars, model, anchor);
    append_five_minute_bars(bars, model, anchor);
}

fn append_weekly_bars(bars: &mut Vec<Bar>, model: &PriceModel, anchor: DateTime<Utc>) {
    for index in 0..WEEKLY_BARS {
        let weeks_ago = i64::try_from(WEEKLY_BARS - index - 1).unwrap_or_default();
        let end = anchor - Duration::weeks(weeks_ago);
        let start = end - Duration::weeks(1);
        bars.push(make_bar(model, "1Week", start, end, anchor, 5.0, index));
    }
}

fn append_daily_bars(bars: &mut Vec<Bar>, model: &PriceModel, anchor: DateTime<Utc>) {
    let dates = trading_dates_ending(anchor.date_naive(), DAILY_BARS + 1);
    for (index, pair) in dates.windows(2).enumerate() {
        let start = at_market_close(pair[0]);
        let end = at_market_close(pair[1]);
        bars.push(make_bar(model, "1Day", start, end, anchor, 1.0, index));
    }
}

fn append_hourly_bars(bars: &mut Vec<Bar>, model: &PriceModel, anchor: DateTime<Utc>) {
    let dates = trading_dates_ending(anchor.date_naive(), HOURLY_SESSIONS);
    for (session_index, date) in dates.into_iter().enumerate() {
        let mut start = at_time(date, 13, 30);
        for hour_index in 0..HOURS_PER_SESSION {
            let end = if hour_index + 1 == HOURS_PER_SESSION {
                at_time(date, 20, 0)
            } else {
                start + Duration::hours(1)
            };
            let index = session_index * HOURS_PER_SESSION + hour_index;
            bars.push(make_bar(
                model,
                "1Hour",
                start,
                end,
                anchor,
                1.0 / 7.0,
                index,
            ));
            start = end;
        }
    }
}

fn append_five_minute_bars(bars: &mut Vec<Bar>, model: &PriceModel, anchor: DateTime<Utc>) {
    let mut start = at_time(anchor.date_naive(), 13, 30);
    for index in 0..FIVE_MINUTE_BARS {
        let end = start + Duration::minutes(5);
        bars.push(make_bar(
            model,
            "5Min",
            start,
            end,
            anchor,
            1.0 / 78.0,
            index,
        ));
        start = end;
    }
}

fn make_bar(
    model: &PriceModel,
    timeframe: &str,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    market_anchor: DateTime<Utc>,
    volume_fraction: f64,
    index: usize,
) -> Bar {
    let open = model.price_days_ago(days_between(market_anchor, start));
    let close = model.price_days_ago(days_between(market_anchor, end));
    let salt = end.timestamp().unsigned_abs()
        ^ hash64(timeframe)
        ^ u64::try_from(index).unwrap_or_default();
    let wick = (0.0015 + unit(model.seed, salt) * 0.007) * volume_fraction.sqrt().max(0.3);
    let activity = 0.68 + unit(model.seed, salt ^ 0x51f1) * 0.75;
    let volume = model.daily_volume * volume_fraction * activity;
    let trade_count = match timeframe {
        "1Week" => model.daily_trades.saturating_mul(5),
        "1Day" => model.daily_trades,
        "1Hour" => model.daily_trades / 7,
        _ => model.daily_trades / 78,
    }
    .max(1);
    let high = open.max(close) * (1.0 + wick);
    let low = open.min(close) * (1.0 - wick);
    Bar {
        symbol: model.symbol.clone(),
        timeframe: timeframe.to_owned(),
        timestamp: end,
        open,
        high,
        low,
        close,
        volume,
        trade_count: Some(trade_count),
        vwap: Some((open + high + low + close) / 4.0),
        source: "demo".to_owned(),
    }
}

fn make_news(
    company: &Company,
    sector_index: usize,
    rank: u16,
    as_of: DateTime<Utc>,
) -> [NewsItem; 2] {
    let sources = ["DemoWire", "Market Ledger", "Terminal Brief"];
    let source_index = (usize::from(rank) + sector_index) % sources.len();
    let first_age = 5 + i64::from(rank % 19);
    let second_age = 31 + i64::from((rank * 3) % 47);
    let base_url = format!("https://example.invalid/stock-tui/{}", company.symbol);
    [
        NewsItem {
            id: format!("demo-{}-outlook", company.symbol),
            headline: format!("{} outlines priorities for the next operating cycle", company.name),
            source: sources[source_index].to_owned(),
            published_at: as_of - Duration::hours(first_age),
            url: format!("{base_url}/outlook"),
            summary: "Simulated offline headline for demonstrating the news reader; this is not a live report."
                .to_owned(),
            symbols: vec![company.symbol.clone()],
        },
        NewsItem {
            id: format!("demo-{}-sector", company.symbol),
            headline: format!(
                "{} investors weigh growth, demand, and margin trends",
                company.sector.map_or("Market", Sector::label)
            ),
            source: sources[(source_index + 1) % sources.len()].to_owned(),
            published_at: as_of - Duration::hours(second_age),
            url: format!("{base_url}/sector-trends"),
            summary: "Deterministic demo content provides a concise related-news row while the app is offline."
                .to_owned(),
            symbols: vec![company.symbol.clone()],
        },
    ]
}

fn last_completed_market_close(as_of: DateTime<Utc>) -> DateTime<Utc> {
    let close = NaiveTime::from_hms_opt(20, 0, 0).unwrap_or(NaiveTime::MIN);
    let mut date = as_of.date_naive();
    if !is_trading_day(date) || as_of.time() < close {
        date = previous_date(date);
    }
    while !is_trading_day(date) {
        date = previous_date(date);
    }
    DateTime::from_naive_utc_and_offset(date.and_time(close), Utc)
}

fn trading_dates_ending(mut date: NaiveDate, count: usize) -> Vec<NaiveDate> {
    let mut dates = Vec::with_capacity(count);
    while dates.len() < count {
        if is_trading_day(date) {
            dates.push(date);
        }
        date = previous_date(date);
    }
    dates.reverse();
    dates
}

fn previous_date(date: NaiveDate) -> NaiveDate {
    date.checked_sub_days(Days::new(1)).unwrap_or(date)
}

fn is_trading_day(date: NaiveDate) -> bool {
    !matches!(date.weekday(), Weekday::Sat | Weekday::Sun)
}

fn at_market_close(date: NaiveDate) -> DateTime<Utc> {
    at_time(date, 20, 0)
}

fn at_time(date: NaiveDate, hour: u32, minute: u32) -> DateTime<Utc> {
    let time = NaiveTime::from_hms_opt(hour, minute, 0).unwrap_or(NaiveTime::MIN);
    DateTime::from_naive_utc_and_offset(date.and_time(time), Utc)
}

fn days_between(anchor: DateTime<Utc>, value: DateTime<Utc>) -> f64 {
    let seconds = anchor.signed_duration_since(value).num_seconds().max(0);
    let bounded = u32::try_from(seconds).unwrap_or(u32::MAX);
    f64::from(bounded) / 86_400.0
}

fn hash64(value: &str) -> u64 {
    value.bytes().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

fn mixed(seed: u64, salt: u64) -> u64 {
    let mut value = seed
        .wrapping_add(salt.wrapping_mul(0x9e37_79b9_7f4a_7c15))
        .wrapping_add(0x9e37_79b9_7f4a_7c15);
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

fn unit(seed: u64, salt: u64) -> f64 {
    let upper = u32::try_from(mixed(seed, salt) >> 32).unwrap_or_default();
    f64::from(upper) / f64::from(u32::MAX)
}

fn generated_identity(sector: Sector, rank: u16) -> (String, String) {
    const ADJECTIVES: [&str; 10] = [
        "Apex",
        "Beacon",
        "Cedar",
        "Crest",
        "Frontier",
        "Harbor",
        "Keystone",
        "Northstar",
        "Pioneer",
        "Summit",
    ];
    let generated_index = usize::from(rank).saturating_sub(featured_companies(sector).len() + 1);
    let businesses = generated_businesses(sector);
    let adjective = ADJECTIVES[(generated_index / businesses.len()) % ADJECTIVES.len()];
    let business = businesses[generated_index % businesses.len()];
    (
        format!("{}{:03}", sector_symbol_prefix(sector), rank),
        format!("{adjective} {business}"),
    )
}

const fn sector_symbol_prefix(sector: Sector) -> &'static str {
    match sector {
        Sector::Consumer => "CS",
        Sector::Services => "SV",
        Sector::Healthcare => "HC",
        Sector::Energy => "EN",
        Sector::Technology => "TC",
        Sector::Financial => "FN",
        Sector::Industrial => "IN",
        Sector::Materials => "MT",
        Sector::Utilities => "UT",
    }
}

const fn sector_market_cap(sector: Sector) -> f64 {
    match sector {
        Sector::Consumer => 900_000_000_000.0,
        Sector::Services => 2_800_000_000_000.0,
        Sector::Healthcare => 850_000_000_000.0,
        Sector::Energy => 520_000_000_000.0,
        Sector::Technology => 3_900_000_000_000.0,
        Sector::Financial => 1_100_000_000_000.0,
        Sector::Industrial => 410_000_000_000.0,
        Sector::Materials => 260_000_000_000.0,
        Sector::Utilities => 190_000_000_000.0,
    }
}

const fn industries(sector: Sector) -> &'static [&'static str; 8] {
    match sector {
        Sector::Consumer => &[
            "Retail",
            "Consumer Products",
            "Food and Beverage",
            "Apparel",
            "Restaurants",
            "Home Improvement",
            "Leisure",
            "Household Products",
        ],
        Sector::Services => &[
            "Interactive Media",
            "Telecommunications",
            "Entertainment",
            "Travel Services",
            "Business Services",
            "Transportation",
            "Hospitality",
            "Digital Commerce",
        ],
        Sector::Healthcare => &[
            "Pharmaceuticals",
            "Managed Care",
            "Medical Devices",
            "Biotechnology",
            "Diagnostics",
            "Life Sciences",
            "Health Services",
            "Medical Technology",
        ],
        Sector::Energy => &[
            "Integrated Energy",
            "Exploration and Production",
            "Oilfield Services",
            "Refining",
            "Midstream",
            "Natural Gas",
            "Energy Equipment",
            "Renewable Energy",
        ],
        Sector::Technology => &[
            "Software",
            "Semiconductors",
            "Cloud Computing",
            "Hardware",
            "Cybersecurity",
            "Data Infrastructure",
            "IT Services",
            "Electronic Design",
        ],
        Sector::Financial => &[
            "Banking",
            "Capital Markets",
            "Insurance",
            "Payments",
            "Asset Management",
            "Consumer Finance",
            "Financial Data",
            "Brokerage",
        ],
        Sector::Industrial => &[
            "Aerospace and Defense",
            "Machinery",
            "Rail Transportation",
            "Logistics",
            "Electrical Equipment",
            "Engineering",
            "Waste Services",
            "Industrial Automation",
        ],
        Sector::Materials => &[
            "Chemicals",
            "Metals and Mining",
            "Steel",
            "Construction Materials",
            "Packaging",
            "Agricultural Inputs",
            "Specialty Materials",
            "Paper Products",
        ],
        Sector::Utilities => &[
            "Electric Utilities",
            "Multi-Utilities",
            "Independent Power",
            "Water Utilities",
            "Natural Gas Utilities",
            "Renewable Utilities",
            "Grid Infrastructure",
            "Regulated Power",
        ],
    }
}

const fn generated_businesses(sector: Sector) -> &'static [&'static str; 8] {
    match sector {
        Sector::Consumer => &[
            "Brands", "Foods", "Home", "Apparel", "Beverage", "Retail", "Leisure", "Markets",
        ],
        Sector::Services => &[
            "Media",
            "Networks",
            "Travel",
            "Logistics",
            "Entertainment",
            "Communications",
            "Hospitality",
            "Commerce",
        ],
        Sector::Healthcare => &[
            "Therapeutics",
            "Medical",
            "Health Systems",
            "Diagnostics",
            "Biopharma",
            "Surgical",
            "Care",
            "Life Sciences",
        ],
        Sector::Energy => &[
            "Energy",
            "Resources",
            "Midstream",
            "Power",
            "Drilling",
            "Petroleum",
            "Gas",
            "Renewables",
        ],
        Sector::Technology => &[
            "Software",
            "Systems",
            "Semiconductor",
            "Cloud",
            "Cybersecurity",
            "Data",
            "Computing",
            "Automation",
        ],
        Sector::Financial => &[
            "Capital",
            "Bancorp",
            "Insurance",
            "Payments",
            "Markets",
            "Advisors",
            "Credit",
            "Financial",
        ],
        Sector::Industrial => &[
            "Industries",
            "Aerospace",
            "Machinery",
            "Transport",
            "Automation",
            "Engineering",
            "Logistics",
            "Manufacturing",
        ],
        Sector::Materials => &[
            "Materials",
            "Mining",
            "Chemicals",
            "Steel",
            "Aggregates",
            "Packaging",
            "Minerals",
            "Specialties",
        ],
        Sector::Utilities => &[
            "Electric",
            "Water",
            "Grid",
            "Utilities",
            "Energy Services",
            "Infrastructure",
            "Power",
            "Gas",
        ],
    }
}

const fn featured_companies(sector: Sector) -> &'static [(&'static str, &'static str)] {
    match sector {
        Sector::Consumer => &[
            ("WMT", "Walmart"),
            ("AMZN", "Amazon.com"),
            ("COST", "Costco Wholesale"),
            ("HD", "Home Depot"),
            ("PG", "Procter & Gamble"),
            ("KO", "Coca-Cola"),
            ("PEP", "PepsiCo"),
            ("MCD", "McDonald's"),
            ("NKE", "Nike"),
            ("LOW", "Lowe's"),
            ("SBUX", "Starbucks"),
            ("TJX", "TJX Companies"),
            ("TGT", "Target"),
            ("MDLZ", "Mondelez International"),
            ("CL", "Colgate-Palmolive"),
            ("PM", "Philip Morris International"),
            ("MO", "Altria Group"),
            ("EL", "Estee Lauder"),
            ("KMB", "Kimberly-Clark"),
            ("ORLY", "O'Reilly Automotive"),
        ],
        Sector::Services => &[
            ("GOOGL", "Alphabet"),
            ("META", "Meta Platforms"),
            ("NFLX", "Netflix"),
            ("DIS", "Walt Disney"),
            ("TMUS", "T-Mobile US"),
            ("VZ", "Verizon Communications"),
            ("T", "AT&T"),
            ("CMCSA", "Comcast"),
            ("UBER", "Uber Technologies"),
            ("BKNG", "Booking Holdings"),
            ("ABNB", "Airbnb"),
            ("SPOT", "Spotify Technology"),
            ("DASH", "DoorDash"),
            ("CHTR", "Charter Communications"),
            ("EA", "Electronic Arts"),
            ("RBLX", "Roblox"),
            ("LYV", "Live Nation Entertainment"),
            ("WBD", "Warner Bros. Discovery"),
            ("TTWO", "Take-Two Interactive"),
            ("ROST", "Ross Stores"),
        ],
        Sector::Healthcare => &[
            ("LLY", "Eli Lilly"),
            ("UNH", "UnitedHealth Group"),
            ("JNJ", "Johnson & Johnson"),
            ("ABBV", "AbbVie"),
            ("MRK", "Merck"),
            ("TMO", "Thermo Fisher Scientific"),
            ("ABT", "Abbott Laboratories"),
            ("AMGN", "Amgen"),
            ("PFE", "Pfizer"),
            ("ISRG", "Intuitive Surgical"),
            ("GILD", "Gilead Sciences"),
            ("DHR", "Danaher"),
            ("BMY", "Bristol-Myers Squibb"),
            ("SYK", "Stryker"),
            ("CVS", "CVS Health"),
            ("MDT", "Medtronic"),
            ("ELV", "Elevance Health"),
            ("REGN", "Regeneron Pharmaceuticals"),
            ("VRTX", "Vertex Pharmaceuticals"),
            ("ZTS", "Zoetis"),
        ],
        Sector::Energy => &[
            ("XOM", "Exxon Mobil"),
            ("CVX", "Chevron"),
            ("COP", "ConocoPhillips"),
            ("SLB", "SLB"),
            ("EOG", "EOG Resources"),
            ("MPC", "Marathon Petroleum"),
            ("PSX", "Phillips 66"),
            ("VLO", "Valero Energy"),
            ("OXY", "Occidental Petroleum"),
            ("KMI", "Kinder Morgan"),
            ("WMB", "Williams Companies"),
            ("LNG", "Cheniere Energy"),
            ("HAL", "Halliburton"),
            ("BKR", "Baker Hughes"),
            ("FANG", "Diamondback Energy"),
            ("DVN", "Devon Energy"),
            ("HES", "Hess"),
            ("CTRA", "Coterra Energy"),
            ("EQT", "EQT"),
            ("TRGP", "Targa Resources"),
        ],
        Sector::Technology => &[
            ("MSFT", "Microsoft"),
            ("AAPL", "Apple"),
            ("NVDA", "NVIDIA"),
            ("AVGO", "Broadcom"),
            ("ORCL", "Oracle"),
            ("CRM", "Salesforce"),
            ("AMD", "Advanced Micro Devices"),
            ("ADBE", "Adobe"),
            ("CSCO", "Cisco Systems"),
            ("IBM", "IBM"),
            ("QCOM", "Qualcomm"),
            ("TXN", "Texas Instruments"),
            ("INTU", "Intuit"),
            ("NOW", "ServiceNow"),
            ("AMAT", "Applied Materials"),
            ("MU", "Micron Technology"),
            ("PANW", "Palo Alto Networks"),
            ("ADI", "Analog Devices"),
            ("KLAC", "KLA"),
            ("SNPS", "Synopsys"),
        ],
        Sector::Financial => &[
            ("BRK.B", "Berkshire Hathaway"),
            ("JPM", "JPMorgan Chase"),
            ("BAC", "Bank of America"),
            ("WFC", "Wells Fargo"),
            ("GS", "Goldman Sachs"),
            ("MS", "Morgan Stanley"),
            ("C", "Citigroup"),
            ("SCHW", "Charles Schwab"),
            ("BLK", "BlackRock"),
            ("AXP", "American Express"),
            ("SPGI", "S&P Global"),
            ("CME", "CME Group"),
            ("ICE", "Intercontinental Exchange"),
            ("CB", "Chubb"),
            ("PGR", "Progressive"),
            ("MMC", "Marsh McLennan"),
            ("COF", "Capital One Financial"),
            ("USB", "U.S. Bancorp"),
            ("PNC", "PNC Financial Services"),
            ("BK", "Bank of New York Mellon"),
        ],
        Sector::Industrial => &[
            ("GE", "GE Aerospace"),
            ("CAT", "Caterpillar"),
            ("RTX", "RTX"),
            ("HON", "Honeywell International"),
            ("UNP", "Union Pacific"),
            ("BA", "Boeing"),
            ("DE", "Deere & Company"),
            ("LMT", "Lockheed Martin"),
            ("UPS", "United Parcel Service"),
            ("ETN", "Eaton"),
            ("ADP", "Automatic Data Processing"),
            ("PH", "Parker-Hannifin"),
            ("GD", "General Dynamics"),
            ("NOC", "Northrop Grumman"),
            ("WM", "Waste Management"),
            ("ITW", "Illinois Tool Works"),
            ("CSX", "CSX"),
            ("EMR", "Emerson Electric"),
            ("FDX", "FedEx"),
            ("MMM", "3M"),
        ],
        Sector::Materials => &[
            ("LIN", "Linde"),
            ("SHW", "Sherwin-Williams"),
            ("FCX", "Freeport-McMoRan"),
            ("NEM", "Newmont"),
            ("APD", "Air Products and Chemicals"),
            ("ECL", "Ecolab"),
            ("NUE", "Nucor"),
            ("DOW", "Dow"),
            ("CTVA", "Corteva"),
            ("DD", "DuPont"),
            ("PPG", "PPG Industries"),
            ("VMC", "Vulcan Materials"),
            ("MLM", "Martin Marietta Materials"),
            ("STLD", "Steel Dynamics"),
            ("ALB", "Albemarle"),
            ("CF", "CF Industries"),
            ("MOS", "Mosaic"),
            ("IFF", "International Flavors & Fragrances"),
            ("IP", "International Paper"),
            ("BALL", "Ball"),
        ],
        Sector::Utilities => &[
            ("NEE", "NextEra Energy"),
            ("SO", "Southern Company"),
            ("DUK", "Duke Energy"),
            ("CEG", "Constellation Energy"),
            ("AEP", "American Electric Power"),
            ("SRE", "Sempra"),
            ("D", "Dominion Energy"),
            ("DTE", "DTE Energy"),
            ("EXC", "Exelon"),
            ("XEL", "Xcel Energy"),
            ("ED", "Consolidated Edison"),
            ("PEG", "Public Service Enterprise Group"),
            ("WEC", "WEC Energy Group"),
            ("PCG", "PG&E"),
            ("EIX", "Edison International"),
            ("AWK", "American Water Works"),
            ("ES", "Eversource Energy"),
            ("FE", "FirstEnergy"),
            ("PPL", "PPL"),
            ("AEE", "Ameren"),
        ],
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};

    use chrono::TimeZone;

    use super::*;

    fn fixed_now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 7, 13, 22, 15, 0)
            .single()
            .expect("valid fixture timestamp")
    }

    #[test]
    fn universe_has_one_hundred_unique_companies_per_sector() {
        let mut companies = Vec::new();
        let mut snapshots = Vec::new();
        for (sector_index, sector) in Sector::ALL.into_iter().enumerate() {
            for rank in 1_u16..=100 {
                let (company, model) = make_company(sector, sector_index, rank, fixed_now());
                snapshots.push(make_snapshot(&model, fixed_now()));
                companies.push(company);
            }
        }

        assert_eq!(companies.len(), 900);
        let unique_symbols = companies
            .iter()
            .map(|company| company.symbol.as_str())
            .collect::<HashSet<_>>();
        assert_eq!(unique_symbols.len(), companies.len());

        let mut sector_counts = HashMap::new();
        for company in &companies {
            *sector_counts.entry(company.sector).or_insert(0) += 1;
            assert!(company.market_cap.is_some_and(|value| value > 0.0));
            assert!(company.shares_outstanding.is_some_and(|value| value > 0.0));
        }
        for sector in Sector::ALL {
            assert_eq!(sector_counts.get(&Some(sector)), Some(&100));
        }
        assert!(companies.iter().any(|company| company.symbol == "AAPL"));
        assert!(companies.iter().any(|company| company.symbol == "JPM"));
        assert!(companies.iter().any(|company| company.symbol == "XOM"));

        let returns = snapshots
            .iter()
            .filter_map(Snapshot::day_return)
            .collect::<Vec<_>>();
        assert!(returns.iter().any(|value| *value < -0.07));
        assert!(returns.iter().any(|value| *value > 0.07));
        assert!(snapshots.iter().all(|snapshot| {
            snapshot
                .volume
                .is_some_and(|volume| volume.is_finite() && volume > 0.0)
        }));
    }

    #[test]
    fn one_symbol_has_history_for_every_range() {
        let (_, model) = make_company(Sector::Technology, 4, 2, fixed_now());
        let anchor = last_completed_market_close(fixed_now());
        let mut bars = Vec::new();
        append_bars(&mut bars, &model, anchor);

        assert_eq!(
            bars.len(),
            FIVE_MINUTE_BARS + HOURLY_SESSIONS * HOURS_PER_SESSION + DAILY_BARS + WEEKLY_BARS
        );
        for (timeframe, expected) in [
            ("5Min", FIVE_MINUTE_BARS),
            ("1Hour", HOURLY_SESSIONS * HOURS_PER_SESSION),
            ("1Day", DAILY_BARS),
            ("1Week", WEEKLY_BARS),
        ] {
            assert_eq!(
                bars.iter().filter(|bar| bar.timeframe == timeframe).count(),
                expected
            );
        }
        let oldest_week = bars
            .iter()
            .find(|bar| bar.timeframe == "1Week")
            .expect("weekly history");
        assert!(anchor.signed_duration_since(oldest_week.timestamp) >= Duration::days(1_826));
        assert!(bars.iter().all(|bar| {
            bar.open.is_finite()
                && bar.high >= bar.open.max(bar.close)
                && bar.low <= bar.open.min(bar.close)
                && bar.volume > 0.0
        }));
    }

    #[test]
    fn scalar_generation_is_repeatable_and_news_is_clearly_simulated() {
        let (first_company, first_model) = make_company(Sector::Healthcare, 2, 17, fixed_now());
        let (second_company, second_model) = make_company(Sector::Healthcare, 2, 17, fixed_now());
        assert_eq!(first_company.symbol, second_company.symbol);
        assert_eq!(first_company.name, second_company.name);
        assert_eq!(
            first_company.market_cap.map(f64::to_bits),
            second_company.market_cap.map(f64::to_bits)
        );
        assert_eq!(
            first_model.price_days_ago(1_826.0).to_bits(),
            second_model.price_days_ago(1_826.0).to_bits()
        );

        let news = make_news(&first_company, 2, 17, fixed_now());
        assert!(
            news.iter()
                .all(|item| item.summary.contains("demo") || item.summary.contains("Simulated"))
        );
        assert!(news.iter().all(|item| {
            item.symbols.len() == 1 && item.symbols.first() == Some(&first_company.symbol)
        }));
    }
}
