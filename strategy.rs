#![allow(dead_code)]

pub const SECS_PER_HOUR: i64 = 3_600;

#[derive(Clone, Debug)]
pub struct Config {
    pub d_min: f64,
    pub p_min: f64,
    pub p_max: f64,
    pub dist_cond_sl_min: f64,
    pub x_pnl_threshold: f64,
    pub expired_hours_min: f64,
    pub expired_hours_max: f64,
    pub pesl_dcoff: f64,
    pub min_x: f64,
    pub max_x: f64,
    pub stake_c: f64,
    pub c_min: f64,
    pub funding_rate: f64,
    pub fee_rate: f64,
    pub max_bid_ask_spread: f64,
    pub price_impact: f64,
    pub parallel_orders_per_market: usize,
    pub concurrent_orders_all_markets: usize,
    pub min_entry_gap_minutes: i64,
    pub tracking_timeout_hours: i64,
    pub max_bank_loss_pct: f64,
    pub deposit_market: f64,
    pub deposit_perp: f64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            d_min: 700.0,
            p_min: 0.0,
            p_max: 0.80,
            dist_cond_sl_min: 400.0,
            x_pnl_threshold: 100.0,
            expired_hours_min: 1.0,
            expired_hours_max: 36.0,
            pesl_dcoff: 100.0,
            min_x: 0.68,
            max_x: 0.98,
            stake_c: 1.0,
            c_min: 1.0,
            funding_rate: 0.000_015,
            fee_rate: 0.07,
            max_bid_ask_spread: 0.06,
            price_impact: 0.01,
            parallel_orders_per_market: 2,
            concurrent_orders_all_markets: 4,
            min_entry_gap_minutes: 30,
            tracking_timeout_hours: 25,
            max_bank_loss_pct: 20.0,
            deposit_market: 500.0,
            deposit_perp: 500.0,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Market {
    pub condition_id: String,
    pub yes_token_id: String,
    pub strike_k: f64,
    pub expiry_ts: i64,
}

#[derive(Clone, Copy, Debug)]
pub struct Level {
    pub price: f64,
    pub size: f64,
}

#[derive(Clone, Debug, Default)]
pub struct Book {
    pub bids: Vec<Level>,
    pub asks: Vec<Level>,
}

impl Book {
    pub fn best_ask(&self) -> Option<f64> {
        self.asks
            .iter()
            .map(|l| l.price)
            .filter(|p| p.is_finite())
            .reduce(f64::min)
    }

    pub fn best_bid(&self) -> Option<f64> {
        self.bids
            .iter()
            .map(|l| l.price)
            .filter(|p| p.is_finite())
            .reduce(f64::max)
    }

    pub fn ask_depth_usd(&self, best_ask: f64, max_impact: f64) -> f64 {
        let limit = best_ask + max_impact;
        self.asks
            .iter()
            .filter(|l| l.price.is_finite() && l.size.is_finite() && l.price <= limit)
            .map(|l| l.price * l.size)
            .sum()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bet {
    pub n: f64,
    pub entry_ask: f64,
    pub fee: f64,
}

#[derive(Clone, Debug)]
pub struct Position {
    pub condition_id: String,
    pub yes_token_id: String,
    pub strike_k: f64,
    pub expiry_ts: i64,
    pub opened_ts: i64,
    pub btc_entry: f64,
    pub d: f64,
    pub q: f64,
    pub bets: Vec<Bet>,
    pub conditional_sl: f64,
    pub stop_loss: Option<f64>,
    pub phase2: bool,
    pub funding_hours: u64,
    pub last_funding_hour: i64,
    pub hedge_closed: bool,
    pub hedge_pnl_locked: f64,
    pub hedge_closed_ts: Option<i64>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Pnl {
    pub market_usd: f64,
    pub hedge_usd: f64,
    pub funding_usd: f64,
    pub fees_usd: f64,
    pub total_usd: f64,
    pub total_pct: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloseReason {
    TakeProfit,
    StopLoss,
    Pesl,
    MarketExpiry,
    TrackingExitHigh,
    TrackingExitLow,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Reject {
    GlobalLimit,
    MarketLimit,
    EntryGap(i64),
    Expired,
    ExpiryWindow(f64),
    BelowStrike,
    Distance(f64),
    NoAsk,
    NoBid,
    CrossedBook,
    AskRange(f64),
    Spread(f64),
    Liquidity(f64),
    CondSlDistance(f64),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Action {
    Hold,
    AwaitBook,
    EnterPhase2 {
        bet: Bet,
        stop_loss: f64,
    },
    CloseHedge {
        hedge_pnl: f64,
    },
    Phase2ThenCloseHedge {
        bet: Bet,
        stop_loss: f64,
        hedge_pnl: f64,
    },
    Phase2ThenClose {
        bet: Bet,
        stop_loss: f64,
        reason: CloseReason,
    },
    Close(CloseReason),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HedgeLeg {
    pub btc_entry: f64,
    pub q: f64,
    pub funding_hours: u64,
    pub funding_rate: f64,
}

pub fn taker_fee(shares: f64, price: f64, fee_rate: f64) -> f64 {
    let fee = shares * fee_rate * price * (1.0 - price);
    if !fee.is_finite() || fee <= 0.0 {
        return 0.0;
    }
    (fee * 1e5).round() / 1e5
}

pub fn compute_pnl(
    bets: &[Bet],
    hedge: &HedgeLeg,
    btc: f64,
    bid: f64,
    stake_c: f64,
    fee_rate: f64,
) -> Pnl {
    pnl_at(bets, hedge, btc, bid, stake_c, fee_rate)
}

pub fn settlement_pnl(cfg: &Config, pos: &Position, btc: f64, yes_won: bool) -> Pnl {
    let hedge = HedgeLeg {
        btc_entry: pos.btc_entry,
        q: pos.q,
        funding_hours: pos.funding_hours,
        funding_rate: cfg.funding_rate,
    };
    let payout = if yes_won { 1.0 } else { 0.0 };
    let pnl = pnl_at(&pos.bets, &hedge, btc, payout, cfg.stake_c, 0.0);
    lock_hedge(cfg, pos, pnl)
}

fn pnl_at(
    bets: &[Bet],
    hedge: &HedgeLeg,
    btc: f64,
    price: f64,
    stake_c: f64,
    exit_fee_rate: f64,
) -> Pnl {
    let HedgeLeg {
        btc_entry,
        q,
        funding_hours,
        funding_rate,
    } = *hedge;
    if !q.is_finite() || q <= 0.0 || !btc.is_finite() || btc <= 0.0 {
        return Pnl::default();
    }
    let shares: f64 = bets.iter().map(|b| b.n).sum();
    let entry_fees: f64 = bets.iter().map(|b| b.fee).sum();
    let fees_usd = entry_fees + taker_fee(shares, price, exit_fee_rate);
    let gross_usd: f64 = bets.iter().map(|b| b.n * (price - b.entry_ask)).sum();
    let market_usd = gross_usd - fees_usd;
    let funding_usd = q * funding_hours as f64 * funding_rate * btc;
    let hedge_usd = -q * (btc - btc_entry) - funding_usd;
    let total_usd = market_usd + hedge_usd;
    let c_total = bets.len() as f64 * stake_c;
    let total_pct = if c_total.abs() > 1e-12 {
        total_usd / c_total * 100.0
    } else {
        0.0
    };
    Pnl {
        market_usd,
        hedge_usd,
        funding_usd,
        fees_usd,
        total_usd,
        total_pct,
    }
}

pub fn position_pnl(cfg: &Config, pos: &Position, btc: f64, bid: f64) -> Pnl {
    let hedge = HedgeLeg {
        btc_entry: pos.btc_entry,
        q: pos.q,
        funding_hours: pos.funding_hours,
        funding_rate: cfg.funding_rate,
    };
    let pnl = compute_pnl(&pos.bets, &hedge, btc, bid, cfg.stake_c, cfg.fee_rate);
    lock_hedge(cfg, pos, pnl)
}

fn lock_hedge(cfg: &Config, pos: &Position, mut pnl: Pnl) -> Pnl {
    if pos.hedge_closed {
        pnl.hedge_usd = pos.hedge_pnl_locked;
        pnl.funding_usd = 0.0;
        pnl.total_usd = pnl.market_usd + pos.hedge_pnl_locked;
        let c_total = pos.bets.len() as f64 * cfg.stake_c;
        pnl.total_pct = if c_total.abs() > 1e-12 {
            pnl.total_usd / c_total * 100.0
        } else {
            0.0
        };
    }
    pnl
}

pub fn evaluate_entry(
    cfg: &Config,
    market: &Market,
    btc: f64,
    book: &Book,
    now_ts: i64,
    open_total: usize,
    open_same_market_ts: &[i64],
) -> Result<Position, Reject> {
    if open_total >= cfg.concurrent_orders_all_markets {
        return Err(Reject::GlobalLimit);
    }
    if open_same_market_ts.len() >= cfg.parallel_orders_per_market {
        return Err(Reject::MarketLimit);
    }
    if let Some(newest) = open_same_market_ts.iter().max() {
        let elapsed_min = (now_ts - *newest) / 60;
        if elapsed_min < cfg.min_entry_gap_minutes {
            return Err(Reject::EntryGap(cfg.min_entry_gap_minutes - elapsed_min));
        }
    }

    if market.expiry_ts <= now_ts {
        return Err(Reject::Expired);
    }
    let hours_left = (market.expiry_ts - now_ts) as f64 / SECS_PER_HOUR as f64;
    if hours_left < cfg.expired_hours_min || hours_left > cfg.expired_hours_max {
        return Err(Reject::ExpiryWindow(hours_left));
    }

    if btc <= market.strike_k {
        return Err(Reject::BelowStrike);
    }
    let d = btc - market.strike_k;
    if d < cfg.d_min {
        return Err(Reject::Distance(d));
    }

    let best_ask = book.best_ask().ok_or(Reject::NoAsk)?;
    let best_bid = book.best_bid().ok_or(Reject::NoBid)?;

    if best_ask < 1e-6 || best_ask < cfg.p_min || best_ask > cfg.p_max {
        return Err(Reject::AskRange(best_ask));
    }
    if best_bid >= best_ask {
        return Err(Reject::CrossedBook);
    }
    let spread = best_ask - best_bid;
    if spread > cfg.max_bid_ask_spread {
        return Err(Reject::Spread(spread));
    }
    let depth = book.ask_depth_usd(best_ask, cfg.price_impact);
    if depth < cfg.c_min {
        return Err(Reject::Liquidity(depth));
    }

    let n = cfg.stake_c / best_ask;
    let fee = taker_fee(n, best_ask, cfg.fee_rate);
    let pi = n - cfg.stake_c - fee;
    let q = cfg.stake_c / d;
    let dist_cond_sl = pi / q;
    if dist_cond_sl < cfg.dist_cond_sl_min {
        return Err(Reject::CondSlDistance(dist_cond_sl));
    }

    Ok(Position {
        condition_id: market.condition_id.clone(),
        yes_token_id: market.yes_token_id.clone(),
        strike_k: market.strike_k,
        expiry_ts: market.expiry_ts,
        opened_ts: now_ts,
        btc_entry: btc,
        d,
        q,
        bets: vec![Bet {
            n,
            entry_ask: best_ask,
            fee,
        }],
        conditional_sl: btc + dist_cond_sl,
        stop_loss: None,
        phase2: false,
        funding_hours: 0,
        last_funding_hour: now_ts / SECS_PER_HOUR,
        hedge_closed: false,
        hedge_pnl_locked: 0.0,
        hedge_closed_ts: None,
    })
}

pub fn accrue_funding(pos: &mut Position, now_ts: i64) {
    let hour = now_ts / SECS_PER_HOUR;
    if hour > pos.last_funding_hour {
        pos.funding_hours += (hour - pos.last_funding_hour) as u64;
        pos.last_funding_hour = hour;
    }
}

pub fn evaluate_position(
    cfg: &Config,
    pos: &Position,
    btc: f64,
    book: &Book,
    now_ts: i64,
) -> Action {
    if now_ts >= pos.expiry_ts {
        return Action::Close(CloseReason::MarketExpiry);
    }
    if !btc.is_finite() || btc <= 0.0 {
        return Action::AwaitBook;
    }

    let bid = match book.best_bid() {
        Some(b) => b,
        None => return Action::AwaitBook,
    };
    let pnl = position_pnl(cfg, pos, btc, bid);
    if !pnl.total_usd.is_finite() || !pnl.total_pct.is_finite() {
        return Action::AwaitBook;
    }

    if pos.hedge_closed {
        if bid > cfg.max_x {
            return Action::Close(CloseReason::TrackingExitHigh);
        }
        if bid < cfg.min_x && bid > 0.0 {
            return Action::Close(CloseReason::TrackingExitLow);
        }
        if let Some(closed_ts) = pos.hedge_closed_ts {
            if (now_ts - closed_ts) / SECS_PER_HOUR >= cfg.tracking_timeout_hours {
                return Action::Close(CloseReason::MarketExpiry);
            }
        }
        return Action::Hold;
    }

    if pnl.total_pct >= cfg.x_pnl_threshold {
        return Action::Close(CloseReason::TakeProfit);
    }

    if pos.phase2 {
        let c_total = pos.bets.len() as f64 * cfg.stake_c;
        let pesl_threshold = c_total * (cfg.pesl_dcoff / 100.0 - 1.0);
        if pnl.total_usd >= pesl_threshold {
            return Action::Close(CloseReason::Pesl);
        }
        if let Some(sl) = pos.stop_loss {
            if btc >= sl {
                if bid >= cfg.min_x && bid <= cfg.max_x {
                    return Action::CloseHedge {
                        hedge_pnl: pnl.hedge_usd,
                    };
                }
                return Action::Close(CloseReason::StopLoss);
            }
        }
        return Action::Hold;
    }

    if btc < pos.conditional_sl {
        return Action::Hold;
    }

    let ask = match book.best_ask() {
        Some(a) if a > 1e-6 => a,
        _ => return Action::AwaitBook,
    };

    let n_new = cfg.stake_c / ask;
    let fee_new = taker_fee(n_new, ask, cfg.fee_rate);
    let n_total: f64 = pos.bets.iter().map(|b| b.n).sum::<f64>() + n_new;
    let fees_total: f64 = pos.bets.iter().map(|b| b.fee).sum::<f64>() + fee_new;
    let c_total = (pos.bets.len() + 1) as f64 * cfg.stake_c;
    let stop_loss = pos.btc_entry + (n_total - c_total - fees_total) / pos.q;
    let bet = Bet {
        n: n_new,
        entry_ask: ask,
        fee: fee_new,
    };

    if btc >= stop_loss {
        if bid >= cfg.min_x && bid <= cfg.max_x {
            return Action::Phase2ThenCloseHedge {
                bet,
                stop_loss,
                hedge_pnl: pnl.hedge_usd,
            };
        }
        return Action::Phase2ThenClose {
            bet,
            stop_loss,
            reason: CloseReason::StopLoss,
        };
    }

    Action::EnterPhase2 { bet, stop_loss }
}

pub fn apply(pos: &mut Position, action: &Action, now_ts: i64) {
    match action {
        Action::EnterPhase2 { bet, stop_loss } => {
            pos.bets.push(*bet);
            pos.stop_loss = Some(*stop_loss);
            pos.phase2 = true;
        }
        Action::CloseHedge { hedge_pnl } => {
            pos.hedge_closed = true;
            pos.hedge_pnl_locked = *hedge_pnl;
            pos.hedge_closed_ts = Some(now_ts);
        }
        Action::Phase2ThenCloseHedge {
            bet,
            stop_loss,
            hedge_pnl,
        } => {
            pos.bets.push(*bet);
            pos.stop_loss = Some(*stop_loss);
            pos.phase2 = true;
            pos.hedge_closed = true;
            pos.hedge_pnl_locked = *hedge_pnl;
            pos.hedge_closed_ts = Some(now_ts);
        }
        Action::Phase2ThenClose { bet, stop_loss, .. } => {
            pos.bets.push(*bet);
            pos.stop_loss = Some(*stop_loss);
            pos.phase2 = true;
        }
        Action::Hold | Action::AwaitBook | Action::Close(_) => {}
    }
}

pub fn deposit_total(cfg: &Config) -> f64 {
    cfg.deposit_market + cfg.deposit_perp
}

pub fn deposit_pnl_pct(cfg: &Config, realized_usd: f64, unrealized_usd: f64) -> f64 {
    let deposit = deposit_total(cfg);
    if deposit.abs() < 1e-12 {
        return 0.0;
    }
    (realized_usd + unrealized_usd) / deposit * 100.0
}

pub fn should_halt(cfg: &Config, deposit_pnl_pct: f64) -> bool {
    deposit_pnl_pct <= -cfg.max_bank_loss_pct
}

#[cfg(test)]
mod tests {
    use super::*;

    fn book(bid: f64, ask: f64, size: f64) -> Book {
        Book {
            bids: vec![Level { price: bid, size }],
            asks: vec![Level { price: ask, size }],
        }
    }

    fn market(k: f64, expiry_ts: i64) -> Market {
        Market {
            condition_id: "c".to_string(),
            yes_token_id: "t".to_string(),
            strike_k: k,
            expiry_ts,
        }
    }

    fn fee_free() -> Config {
        Config {
            fee_rate: 0.0,
            ..Config::default()
        }
    }

    #[test]
    fn taker_fee_follows_the_published_curve() {
        assert!((taker_fee(100.0, 0.50, 0.07) - 1.75).abs() < 1e-9);
        assert!((taker_fee(100.0, 0.30, 0.07) - taker_fee(100.0, 0.70, 0.07)).abs() < 1e-9);
        assert!((taker_fee(1.0, 0.999, 0.07) - 0.00007).abs() < 1e-12);
        assert_eq!(taker_fee(0.01, 0.999, 0.07), 0.0);
        assert_eq!(taker_fee(100.0, 0.50, 0.0), 0.0);
    }

    #[test]
    fn entry_fee_shrinks_the_conditional_sl_distance() {
        let m = market(79_000.0, 24 * SECS_PER_HOUR);
        let b = book(0.59, 0.60, 500.0);
        let free = evaluate_entry(&fee_free(), &m, 80_000.0, &b, 0, 0, &[]).unwrap();
        let paid = evaluate_entry(&Config::default(), &m, 80_000.0, &b, 0, 0, &[]).unwrap();
        assert!((paid.bets[0].fee - 0.028).abs() < 1e-9);
        let expected = free.conditional_sl - paid.bets[0].fee / paid.q;
        assert!((paid.conditional_sl - expected).abs() < 1e-6);
    }

    #[test]
    fn mark_to_market_nets_entry_and_exit_fees() {
        let cfg = Config::default();
        let m = market(79_000.0, 24 * SECS_PER_HOUR);
        let pos = evaluate_entry(&cfg, &m, 80_000.0, &book(0.59, 0.60, 500.0), 0, 0, &[]).unwrap();
        let pnl = position_pnl(&cfg, &pos, 80_000.0, 0.60);
        let exit_fee = taker_fee(pos.bets[0].n, 0.60, cfg.fee_rate);
        assert!((pnl.fees_usd - (pos.bets[0].fee + exit_fee)).abs() < 1e-9);
        assert!((pnl.total_usd + pnl.fees_usd).abs() < 1e-9);
    }

    #[test]
    fn settlement_charges_no_exit_fee() {
        let cfg = Config::default();
        let m = market(79_000.0, 24 * SECS_PER_HOUR);
        let pos = evaluate_entry(&cfg, &m, 80_000.0, &book(0.59, 0.60, 500.0), 0, 0, &[]).unwrap();
        let won = settlement_pnl(&cfg, &pos, 80_000.0, true);
        assert!((won.fees_usd - pos.bets[0].fee).abs() < 1e-9);
        assert!((won.market_usd - (pos.bets[0].n - cfg.stake_c - pos.bets[0].fee)).abs() < 1e-9);
        let lost = settlement_pnl(&cfg, &pos, 80_000.0, false);
        assert!((lost.market_usd + cfg.stake_c + pos.bets[0].fee).abs() < 1e-9);
    }

    #[test]
    fn entry_sets_conditional_sl_from_distance_and_ask() {
        let cfg = fee_free();
        let now = 0;
        let m = market(79_000.0, 24 * SECS_PER_HOUR);
        let pos =
            evaluate_entry(&cfg, &m, 80_000.0, &book(0.59, 0.60, 500.0), now, 0, &[]).unwrap();
        assert!((pos.q - 1.0 / 1000.0).abs() < 1e-12);
        assert!((pos.conditional_sl - (80_000.0 + 1000.0 * 0.4 / 0.6)).abs() < 1e-6);
    }

    #[test]
    fn entry_rejects_thin_conditional_sl_distance() {
        let cfg = Config::default();
        let m = market(79_200.0, 24 * SECS_PER_HOUR);
        let err =
            evaluate_entry(&cfg, &m, 80_000.0, &book(0.74, 0.75, 500.0), 0, 0, &[]).unwrap_err();
        assert!(matches!(err, Reject::CondSlDistance(_)));
    }

    #[test]
    fn entry_rejects_below_strike() {
        let cfg = Config::default();
        let m = market(81_000.0, 24 * SECS_PER_HOUR);
        let err =
            evaluate_entry(&cfg, &m, 80_000.0, &book(0.59, 0.60, 500.0), 0, 0, &[]).unwrap_err();
        assert_eq!(err, Reject::BelowStrike);
    }

    #[test]
    fn crossing_conditional_sl_opens_phase_two() {
        let cfg = Config::default();
        let m = market(79_000.0, 24 * SECS_PER_HOUR);
        let pos = evaluate_entry(&cfg, &m, 80_000.0, &book(0.59, 0.60, 500.0), 0, 0, &[]).unwrap();
        let act = evaluate_position(
            &cfg,
            &pos,
            pos.conditional_sl + 1.0,
            &book(0.89, 0.90, 500.0),
            60,
        );
        assert!(matches!(act, Action::EnterPhase2 { .. }));
    }

    #[test]
    fn stop_loss_inside_bid_band_closes_hedge_only() {
        let cfg = Config::default();
        let m = market(79_000.0, 24 * SECS_PER_HOUR);
        let mut pos =
            evaluate_entry(&cfg, &m, 80_000.0, &book(0.59, 0.60, 500.0), 0, 0, &[]).unwrap();
        let b = book(0.89, 0.90, 500.0);
        let act = evaluate_position(&cfg, &pos, pos.conditional_sl + 1.0, &b, 60);
        apply(&mut pos, &act, 60);
        let sl = pos.stop_loss.unwrap();
        let act2 = evaluate_position(&cfg, &pos, sl + 1.0, &b, 120);
        assert!(matches!(act2, Action::CloseHedge { .. }));
    }

    #[test]
    fn pnl_is_zero_for_degenerate_hedge_size() {
        let hedge = HedgeLeg {
            btc_entry: 80_000.0,
            q: 0.0,
            funding_hours: 0,
            funding_rate: 0.0,
        };
        let p = compute_pnl(
            &[Bet {
                n: 1.0,
                entry_ask: 0.5,
                fee: 0.0,
            }],
            &hedge,
            80_000.0,
            0.6,
            1.0,
            0.07,
        );
        assert_eq!(p, Pnl::default());
    }

    #[test]
    fn tracking_exits_when_bid_leaves_band() {
        let cfg = Config::default();
        let m = market(79_000.0, 24 * SECS_PER_HOUR);
        let mut pos =
            evaluate_entry(&cfg, &m, 80_000.0, &book(0.59, 0.60, 500.0), 0, 0, &[]).unwrap();
        pos.hedge_closed = true;
        pos.hedge_closed_ts = Some(0);
        let act = evaluate_position(&cfg, &pos, 80_000.0, &book(0.99, 0.995, 500.0), 60);
        assert_eq!(act, Action::Close(CloseReason::TrackingExitHigh));
    }
}
