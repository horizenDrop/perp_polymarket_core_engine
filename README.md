# perp_polymarket_core_engine

The decision core of a delta-neutral-by-design strategy on Polymarket BTC binary markets, hedged with a short BTC perpetual on a perp DEX.

This is the strategy extracted from a larger paper-trading bot: entry filters, position formulas, PnL and the full state machine, with every side effect removed. No HTTP, no exchange clients, no persistence, no UI, no logging. A single source file, no dependencies, no `unsafe`.

`strategy.rs` is pure functions over plain structs. You feed it a BTC price, a market and an order book; it returns a `Position` or a `Reject`, and on every later tick an `Action`. What you do with that action is yours to decide.

> **Read [Results](#results) before using any of this.** The structure documented here does not have a positive expectancy. It is published as a worked example of a strategy and of how to take one apart, not as something to trade.

---

## The trade

Polymarket lists binary markets of the form *"Will the price of Bitcoin be above $K on DATE?"*. A YES token settles at $1.00 if BTC is above `K` at expiry and $0.00 otherwise.

The strategy buys YES while BTC is **already above** the strike, and shorts BTC on a perpetual to cancel the directional exposure:

| Leg | Venue | Size |
|---|---|---|
| Long YES | Polymarket | `n = C / ask` tokens for `C` dollars |
| Short BTC | perp DEX | `Q = C / D` BTC, where `D = BTC − K` |

The intended source of return is Polymarket repricing the YES token toward $1.00 faster than the hedge bleeds.

## Entry

`evaluate_entry` applies these checks in exactly this order and returns the first failure as a typed `Reject`:

| # | Check | Default | Reject |
|---|---|---|---|
| 1 | Open positions across all markets | `< 4` | `GlobalLimit` |
| 2 | Open positions in this market | `< 2` | `MarketLimit` |
| 3 | Time since the newest entry in this market | `>= 30 min` | `EntryGap` |
| 4 | Market not expired | `expiry > now` | `Expired` |
| 5 | Time to expiry | `1 … 36 h` | `ExpiryWindow` |
| 6 | BTC strictly above strike | `BTC > K` | `BelowStrike` |
| 7 | Distance to strike | `D >= $700` | `Distance` |
| 8 | Ask side has a price | present | `NoAsk` |
| 9 | Bid side has a price | present | `NoBid` |
| 10 | YES best ask | `>= 1e-6`, within `0.00 … 0.80` | `AskRange` |
| 11 | Book not crossed | `bid < ask` | `CrossedBook` |
| 12 | Bid-ask spread | `<= 0.06` | `Spread` |
| 13 | Depth within `best_ask + 0.01` | `>= $1` | `Liquidity` |
| 14 | Conditional-SL distance | `>= $400` | `CondSlDistance` |

Checks 1 to 3 read counts and timestamps that the **caller** supplies, so what counts as an open position is the caller's policy. In the bot this module came from, Phase-2 bets were not counted toward either limit.

## Formulas

```text
fee(n, p) = round5(n · fee_rate · p · (1 − p))   Polymarket taker fee, USDC

n        = C / ask                        YES tokens bought
pi       = n − C − fee(n, ask)            maximum profit on the YES leg, net of the entry fee
Q        = C / D                          hedge size in BTC
dist_csl = pi / Q = D · (1 − ask) · (1/ask − fee_rate)   distance to the Phase-2 trigger

conditional_sl = BTC_entry + dist_csl
stop_loss      = BTC_entry + (n_total − c_total − fees_total) / Q

market_pnl = Σ nᵢ · (bid − askᵢ) − Σ feeᵢ − fee(n_total, bid)
hedge_pnl  = −Q · (BTC − BTC_entry) − Q · hours · funding_rate · BTC
total_pnl  = market_pnl + hedge_pnl
total_pct  = total_pnl / c_total · 100
```

`conditional_sl` is the BTC price at which the hedge loss exactly cancels the best case on the YES leg. It is an accounting identity, not a risk budget.

## Fees

Since 2026 Polymarket charges takers `shares · fee_rate · p · (1 − p)` USDC on every fill in fee-enabled categories, rounded to 5 decimals. The rate for Crypto markets is `0.07`, which peaks at $1.75 per 100 shares at `p = 0.50`. Makers pay nothing and receive 20% of the taker fees collected in their market as a daily rebate. Redeeming a resolved market is not a trade and carries no fee.

Every leg of this strategy is a taker order, so the module charges the fee on each buy and, in mark-to-market PnL, on selling the whole YES leg at the bid. `position_pnl` is therefore a liquidation value, and the take-profit and break-even exits fire on net numbers. At expiry use `settlement_pnl`, which pays out $1 or $0 per token with no exit fee. Set `fee_rate` to `0.0` for a fee-free market or to reproduce the pre-fee model.

## Position lifecycle

```text
              BTC >= conditional_sl              BTC >= stop_loss
 [Phase 1] ──────────────────────────▶ [Phase 2] ─────────────────────▶ exit
                                                   │
                                                   ├─ bid in [min_x, max_x]
                                                   │    → close hedge only,
                                                   │      carry YES to expiry
                                                   └─ otherwise → close both
```

**Phase 2** adds a second `C`-sized YES bet at the current ask and widens the stop. `Q` is never changed, in Phase 2 or after.

**Tracking mode** starts when the stop fires while the YES bid sits inside `[min_x, max_x]`. The hedge is closed, its PnL is frozen into `hedge_pnl_locked`, and the YES leg is carried alone.

`evaluate_position` resolves in this precedence, top to bottom:

| # | Condition | Result |
|---|---|---|
| 1 | `now >= expiry` | `Close(MarketExpiry)` |
| 2 | BTC price not usable | `AwaitBook` |
| 3 | No best bid, or PnL not finite | `AwaitBook` |
| 4 | Tracking, `bid > max_x` | `Close(TrackingExitHigh)` |
| 5 | Tracking, `0 < bid < min_x` | `Close(TrackingExitLow)` |
| 6 | Tracking, `tracking_timeout_hours` since the hedge closed | `Close(MarketExpiry)` |
| 7 | Tracking, none of the above | `Hold` |
| 8 | `total_pct >= x_pnl_threshold` | `Close(TakeProfit)` |
| 9 | Phase 2, `total_usd >= c_total · (pesl_dcoff/100 − 1)` | `Close(Pesl)` |
| 10 | Phase 2, `BTC >= stop_loss`, bid in band | `CloseHedge` |
| 11 | Phase 2, `BTC >= stop_loss`, bid outside band | `Close(StopLoss)` |
| 12 | Phase 2, otherwise | `Hold` |
| 13 | Phase 1, `BTC < conditional_sl` | `Hold` |
| 14 | Phase 1, trigger reached, no usable ask | `AwaitBook` |
| 15 | Phase 1, trigger reached, `BTC >= stop_loss`, bid in band | `Phase2ThenCloseHedge` |
| 16 | Phase 1, trigger reached, `BTC >= stop_loss`, bid outside band | `Phase2ThenClose` |
| 17 | Phase 1, trigger reached | `EnterPhase2` |

Rows 15 and 16 are the single-tick case: BTC clears `conditional_sl` and the freshly computed `stop_loss` inside one interval. The position still enters Phase 2 before it exits.

`AwaitBook` means the book gave no usable price this tick. Treat it as "do nothing, retry", never as a default value: substituting `0.0` for a missing bid reads as "below `min_x`" and closes both legs at the worst possible moment.

## Parameters

| Key | Default | Meaning |
|---|---|---|
| `d_min` | `700.0` | Minimum `BTC − K`, USD |
| `p_min` / `p_max` | `0.0` / `0.80` | Accepted YES ask band |
| `dist_cond_sl_min` | `400.0` | Minimum distance to the Phase-2 trigger |
| `x_pnl_threshold` | `100.0` | Position close threshold: exit once total PnL reaches this % of the capital staked in that position |
| `expired_hours_min` / `_max` | `1.0` / `36.0` | Expiry window |
| `pesl_dcoff` | `100.0` | Phase-2 early exit at `c_total · (pesl_dcoff/100 − 1)`; `100` is break-even |
| `min_x` / `max_x` | `0.68` / `0.98` | YES bid band checked **at the stop**, not at Phase-2 entry |
| `stake_c` | `1.0` | Stake per bet, USD |
| `c_min` | `1.0` | Minimum fill inside the price-impact window |
| `funding_rate` | `0.000015` | Perp funding per hour, as a fraction |
| `fee_rate` | `0.07` | Polymarket taker fee rate (Crypto category); `0.0` disables fees |
| `max_bid_ask_spread` | `0.06` | Maximum YES spread |
| `price_impact` | `0.01` | Asks counted only up to `best_ask + this` |
| `parallel_orders_per_market` | `2` | Open positions per market |
| `concurrent_orders_all_markets` | `4` | Open positions overall |
| `min_entry_gap_minutes` | `30` | Gap between entries into the same market |
| `tracking_timeout_hours` | `25` | Forced exit from tracking mode |
| `max_bank_loss_pct` | `20.0` | Halt threshold on deposit drawdown |
| `deposit_market` / `deposit_perp` | `500.0` / `500.0` | Balances behind `deposit_pnl_pct` |

## Using it

```rust
let cfg = Config::default();

let mut position = match evaluate_entry(
    &cfg, &market, btc, &book, now_ts, open_total, &same_market_ts,
) {
    Ok(p) => p,
    Err(reason) => return log_skip(reason),
};

loop {
    accrue_funding(&mut position, now_ts);
    let action = evaluate_position(&cfg, &position, btc, &book, now_ts);
    apply(&mut position, &action, now_ts);

    if let Action::Close(reason) = action {
        let pnl = match reason {
            CloseReason::MarketExpiry if now_ts >= position.expiry_ts => {
                settlement_pnl(&cfg, &position, btc, btc > position.strike_k)
            }
            _ => position_pnl(&cfg, &position, btc, bid),
        };
        settle(&position, reason, pnl);
        break;
    }
}
```

`MarketExpiry` is also returned by the tracking-mode timeout, which is a sale at the bid and not a settlement, hence the expiry check above.

You supply the BTC price, the market list, order books, and a clock in Unix seconds. `evaluate_position` expects to run on a short interval; the original ran it every 3 seconds.

`apply` performs the state transition an `Action` implies. It deliberately does **not** close positions: `Phase2ThenClose` pushes the Phase-2 bet and leaves the close to you, and removing a closed position from your own book is your job.

PnL can also be computed without a `Position`, by describing the hedge directly:

```rust
let hedge = HedgeLeg { btc_entry, q, funding_hours, funding_rate };
let pnl = compute_pnl(&bets, &hedge, btc, bid, stake_c, fee_rate);
```

There is no `Cargo.toml` on purpose. Drop the file into an existing crate as a module, or build it standalone:

```bash
rustc --edition 2021 --crate-type lib strategy.rs
rustc --edition 2021 --test strategy.rs -o strategy_tests && ./strategy_tests
```

Verified on rustc 1.100.0: builds with no warnings, `clippy::all` clean, `rustfmt --check` clean, 11 unit tests covering entry sizing, the rejection ladder, the Phase-2 transition, the hedge-only stop, tracking exits, the fee curve, the fee-adjusted trigger, net mark-to-market and settlement.

## Results

The structure was analysed by Monte Carlo simulation and against live Polymarket and Binance data. It does not have a positive expectancy. The reasons are structural rather than implementation bugs, and they are worth more than the code.

Unless stated otherwise, simulations model BTC as a driftless geometric Brownian motion at 50% annualised volatility on a 24-hour expiry, with the YES token priced at Black-Scholes fair value throughout, so there is no mispricing to capture by construction.

**It is not delta neutral.** `Q = C / D` is loss-recovery sizing, not a delta. The dollar delta of the YES leg divided by the dollar delta of the hedge is `d · φ(d) / N(d)`, which never exceeds `0.30` anywhere in the accepted ask band, so the hedge is 3 to 6 times oversized. At `d_min = 700` with BTC at $100,000, every `$1` bet carries `$142.86` of BTC notional, and the YES leg offsets only a fraction of it. Sweeping the drift in simulation put the net short exposure at roughly `$87` per `$1` of stake, averaged over the life of a trade. The result behaves like a leveraged BTC short, not an arbitrage.

**The barriers sit at a fraction of one standard deviation.** Dividing the trigger distance by the dollar standard deviation of the horizon gives `d · (1 − ask) / ask`, which depends only on the ask and peaks near `0.22`. A driftless path touches a barrier that close roughly 80% of the time.

**Phase 2 is a martingale.** Over 12,000 paths it moved mean PnL from `−$0.061` to `−$0.097` per trade while lifting the win rate from 33% to 53%. It buys a better-looking number with a worse expectancy.

**Tracking mode whipsaws.** The hedge is closed at the exact moment BTC has run against the position. In 41.8% of those 12,000 paths the YES leg then collapsed too, exiting below `min_x` at an average of `−$1.08` and turning a hedged loss into a double loss.

**There is no edge term anywhere.** Nothing in the module computes a fair probability; the only cheapness test is `ask <= 0.80`. Over 20,000 paths the strategy returns `−$0.075` per trade, and `+$0.004` with all costs zeroed. The structure is a fair game and the entire loss is the round-trip spread. Breaking even needs about 3 cents of genuine mispricing per bet, which is what `−$0.0003` per trade at a 3-cent edge works out to.

**The position is short volatility, and on the wrong side of it.** Buying an in-the-money binary and hedging it is a short-vol trade: negative gamma, negative vega, high win rate, fat left tail. Measured on 2026-09-20 with BTC at $80,857, the only Polymarket expiry inside the 1 to 36 hour window priced at 19.2% implied volatility, and the next one out, at 37.3 hours, at 27.3%. BTC realized 20.7% over the preceding 24 hours and between 34.6% and 38.6% on every lookback from 7 to 90 days. Each expiry also showed a smile, 26% to 31% near the money against 32% to 39% in the wings, and the `ask <= 0.80` filter lands near its bottom. That is a single snapshot taken on an unusually calm day and is not a statistical result on its own, but the sign of the gap is not marginal.

**Taker fees roughly double the loss.** The numbers above predate Polymarket's taker fee. A re-run with the fee modelled (20,000 driftless paths, 50% volatility, fair-value quotes with a 2-cent spread, 2-minute steps) moved mean PnL per trade from `−$0.054` to `−$0.114` with BTC at $100,000 on a 24-hour expiry, and from `−$0.058` to `−$0.110` at $69,000 on 36 hours. Fees average `$0.055` per trade, because a position that reaches Phase 2 pays the fee on two buys and on the sale. The entry mispricing needed to break even rises from about 4 cents to about 8 cents per share. Without Phase 2 the loss is `−$0.037` before fees and `−$0.075` after.

**The dollar thresholds no longer fit the price.** `dist_csl` cannot exceed roughly `0.22` standard deviations of the horizon. With BTC at $69,000, 50% volatility and a 24-hour expiry that ceiling is about $395 at the ask and $375 once the entry fee is netted out, below `dist_cond_sl_min = 400`, so no market can pass the entry filter at all. It reopens only with more volatility or a longer horizon: $461 at 36 hours.

**A win rate is a dial, not an edge.** Any barrier structure can be tuned to any win rate by moving the barriers. Expectancy and the shape of the tail are the numbers that decide.

## License

MIT. See [LICENSE](LICENSE).

## Status

Extracted and published as-is, not maintained. The project it came from is [polymarket-live-tests](https://github.com/horizenDrop/polymarket-live-tests).
