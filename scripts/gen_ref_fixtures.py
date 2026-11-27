#!/usr/bin/env python3
"""Generate pandas/NumPy reference CSV fixtures for LangMelogne Tier 1 operators."""

from pathlib import Path
import numpy as np
import pandas as pd


def make_bars(n: int = 5000, seed: int = 20261126) -> pd.DataFrame:
    rng = np.random.default_rng(seed)
    rets = rng.normal(loc=0.0002, scale=0.012, size=n)
    close = 100.0 * np.exp(np.cumsum(rets))
    open_ = np.empty(n, dtype=np.float64)
    open_[0] = close[0] * 0.995  # Ensure close[0] > open[0] at bar 0
    open_[1:] = close[:-1] * (1.0 + rng.normal(0.0, 0.003, size=n - 1))

    spread = np.abs(rng.normal(0.006, 0.004, size=n)) * np.maximum(open_, close)
    high = np.maximum(open_, close) + spread
    low = np.minimum(open_, close) - np.abs(rng.normal(0.006, 0.004, size=n)) * np.minimum(open_, close)
    low = np.maximum(low, 0.01)
    volume = rng.integers(1000, 50000, size=n).astype(np.float64)

    # Flat stretch: bars 2000..2100 have identical OHLC prices
    flat_price = close[1999]
    open_[2000:2100] = flat_price
    high[2000:2100] = flat_price
    low[2000:2100] = flat_price
    close[2000:2100] = flat_price

    # NaN holes well after initial warm-up windows
    for idx in [200, 201, 202, 1500, 3200, 3201]:
        close[idx] = np.nan
    open_[500] = np.nan
    high[900] = np.nan
    low[900] = np.nan
    volume[1200] = np.nan
    volume[2500] = 0.0

    return pd.DataFrame(
        {
            "open": open_,
            "high": high,
            "low": low,
            "close": close,
            "volume": volume,
        }
    )


def ewm(s: pd.Series, alpha: float, min_periods: int) -> pd.Series:
    return s.ewm(alpha=alpha, adjust=False, ignore_na=True, min_periods=min_periods).mean()


def ema(s: pd.Series, n: int) -> pd.Series:
    return ewm(s, 2.0 / (n + 1.0), n)


def rma(s: pd.Series, n: int) -> pd.Series:
    return ewm(s, 1.0 / n, n)


def rsi(s: pd.Series, n: int) -> pd.Series:
    delta = s.diff(1)
    gain = delta.clip(lower=0.0)
    loss = (-delta).clip(lower=0.0)
    ag = rma(gain, n)
    al = rma(loss, n)
    out = pd.Series(np.nan, index=s.index, dtype=np.float64)
    valid = ag.notna() & al.notna()
    zero_both = valid & (ag == 0.0) & (al == 0.0)
    zero_loss = valid & (al == 0.0) & (ag != 0.0)
    normal = valid & (al != 0.0)
    out[zero_both] = 50.0
    out[zero_loss] = 100.0
    out[normal] = 100.0 - 100.0 / (1.0 + ag[normal] / al[normal])
    return out


def true_range(df: pd.DataFrame) -> pd.Series:
    pc = df["close"].shift(1)
    tr1 = df["high"] - df["low"]
    tr2 = (df["high"] - pc).abs()
    tr3 = (df["low"] - pc).abs()
    both = pd.concat([tr1, tr2, tr3], axis=1)
    res = both.max(axis=1, skipna=False)
    return res


def linreg_arr(w: np.ndarray) -> tuple[float, float]:
    n = len(w)
    x = np.arange(n, dtype=np.float64)
    mx = (n - 1.0) / 2.0
    my = float(np.mean(w))
    dx = x - mx
    den = float(np.sum(dx * dx))
    if den == 0.0:
        return (np.nan, np.nan)
    slope = float(np.sum(dx * (w - my)) / den)
    intercept = my - slope * mx
    return (slope, intercept)


def two_pass_cov(x: np.ndarray, y: np.ndarray) -> float:
    mx = float(np.mean(x))
    my = float(np.mean(y))
    return float(np.sum((x - mx) * (y - my)) / (len(x) - 1))


def two_pass_corr(x: np.ndarray, y: np.ndarray) -> float:
    mx = float(np.mean(x))
    my = float(np.mean(y))
    dx = x - mx
    dy = y - my
    den = float(np.sqrt(np.sum(dx * dx) * np.sum(dy * dy)))
    if den == 0.0:
        return np.nan
    return float(np.sum(dx * dy) / den)


def rolling_pair(s1: pd.Series, s2: pd.Series, n: int, fn) -> pd.Series:
    a = s1.to_numpy(dtype=np.float64)
    b = s2.to_numpy(dtype=np.float64)
    out = np.full(len(a), np.nan, dtype=np.float64)
    for t in range(n - 1, len(a)):
        wa = a[t + 1 - n : t + 1]
        wb = b[t + 1 - n : t + 1]
        if np.all(np.isfinite(wa)) and np.all(np.isfinite(wb)):
            out[t] = fn(wa, wb)
    return pd.Series(out, index=s1.index)


def compute_crossover(a: pd.Series, b: pd.Series, cross_up: bool) -> pd.Series:
    a0 = a.shift(1)
    b0 = b.shift(1)
    valid = a0.notna() & b0.notna() & a.notna() & b.notna()
    out = pd.Series(np.nan, index=a.index, dtype=np.float64)
    if cross_up:
        cond = (a0 <= b0) & (a > b)
    else:
        cond = (a0 >= b0) & (a < b)
    out[valid] = np.where(cond[valid], 1.0, 0.0)
    return out


def compute_since(cond_vals: pd.Series) -> pd.Series:
    out = np.full(len(cond_vals), np.nan, dtype=np.float64)
    last_true = None
    for i, v in enumerate(cond_vals):
        if pd.isna(v):
            if last_true is not None:
                last_true += 1
            out[i] = np.nan
        elif v == 1.0:
            last_true = 0
            out[i] = 0.0
        else:
            if last_true is not None:
                last_true += 1
                out[i] = float(last_true)
    return pd.Series(out, index=cond_vals.index)


def main() -> None:
    root = Path(__file__).resolve().parent.parent
    out_dir = root / "mln-std" / "tests" / "fixtures" / "ref"
    out_dir.mkdir(parents=True, exist_ok=True)

    df = make_bars()
    df.to_csv(out_dir / "bars.csv", index=False, float_format="%.17g")

    o, h, l, c, v = df["open"], df["high"], df["low"], df["close"], df["volume"]
    cond_up = pd.Series(
        np.where(c.notna() & o.notna(), np.where(c > o, 1.0, 0.0), np.nan),
        index=df.index,
    )

    ema12 = ema(c, 12)
    ema26 = ema(c, 26)
    macd_line = ema12 - ema26
    macd_sig = ema(macd_line, 9)

    weights10 = np.arange(1, 11, dtype=np.float64)
    denom10 = float(np.sum(weights10))

    std20 = c.rolling(20).apply(lambda w: float(np.std(w, ddof=1)), raw=True)
    var20 = c.rolling(20).apply(lambda w: float(np.var(w, ddof=1)), raw=True)

    series_map: dict[str, pd.Series] = {
        "abs": (c - o).abs(),
        "sqrt": np.sqrt(c),
        "log": np.log(c),
        "exp": np.exp((c - o) / o),
        "min": np.minimum(o, c),
        "max": np.maximum(o, c),
        "where": pd.Series(
            np.where(cond_up.isna(), np.nan, np.where(cond_up == 1.0, h, l)),
            index=df.index,
        ),
        "clamp": pd.Series(
            np.where(
                o.notna() & l.notna() & h.notna() & (l <= h),
                np.clip(o + 1.0, l, h),
                np.nan,
            ),
            index=df.index,
        ),
        "mod": c % 5.0,
        "diff": c.diff(1),
        "pct_change": c.pct_change(1, fill_method=None),
        "cumsum": v.cumsum(),
        "sum": v.rolling(20).sum(),
        "mean": c.rolling(20).mean(),
        "sma": c.rolling(20).mean(),
        "std": std20,
        "var": var20,
        "ema": ema12,
        "rma": rma(c, 14),
        "wma": c.rolling(10).apply(lambda w: float(np.dot(w, weights10) / denom10), raw=True),
        "highest": h.rolling(20).max(),
        "lowest": l.rolling(20).min(),
        "rsi": rsi(c, 14),
        "tr": true_range(df),
        "atr": rma(true_range(df), 14),
        "macd": macd_line - macd_sig,
        "bollinger": c.rolling(20).mean() + 2.0 * std20,
        "corr": rolling_pair(c, v, 20, two_pass_corr),
        "cov": rolling_pair(c, v, 20, two_pass_cov),
        "linreg_slope": c.rolling(20).apply(lambda w: linreg_arr(w)[0], raw=True),
        "linreg_intercept": c.rolling(20).apply(lambda w: linreg_arr(w)[1], raw=True),
        "percentile_rank": c.rolling(20).apply(
            lambda w: float(np.sum(w <= w[-1]) / len(w) * 100.0), raw=True
        ),
        "crossover": compute_crossover(ema12, ema26, True),
        "crossunder": compute_crossover(ema12, ema26, False),
        "since": compute_since(cond_up),
    }

    for name, s in series_map.items():
        pd.DataFrame({"value": s}).to_csv(
            out_dir / f"{name}.csv", index=False, float_format="%.17g"
        )
    print(f"Wrote bars.csv + {len(series_map)} operator fixtures to {out_dir}")


if __name__ == "__main__":
    main()
