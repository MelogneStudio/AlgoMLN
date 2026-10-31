#!/usr/bin/env python3
"""Generate indicator golden CSVs from the bundled NIFTY data using pandas."""
from pathlib import Path
import pandas as pd
import numpy as np

ROOT = Path(__file__).resolve().parents[1]
source = ROOT / "sample-data" / "nifty_1min.csv"
out = ROOT / "src" / "indicators" / "tests" / "fixtures"
out.mkdir(parents=True, exist_ok=True)
df = pd.read_csv(source, parse_dates=["datetime"])
close, open_, high, low = (df.close.to_numpy(), df.open.to_numpy(),
                           df.high.to_numpy(), df.low.to_numpy())
# Correct five source rounding discrepancies where close lies less than 0.001
# outside the quoted low/high values before building validated OHLC candles.
high, low = np.maximum(high, close), np.minimum(np.minimum(low, open_), close)
prev = np.roll(close, 1)
tr = np.maximum.reduce((high - low, np.abs(high - prev), np.abs(low - prev)))
tr[0] = np.nan
change = np.diff(close, prepend=np.nan)
gain, loss = np.maximum(change, 0), np.maximum(-change, 0)

def wilders(series, period):
    result = np.full(len(series), np.nan)
    if len(series) <= period:
        return result
    result[period] = np.mean(series[1:period + 1])
    for i in range(period + 1, len(series)):
        result[i] = (result[i - 1] * (period - 1) + series[i]) / period
    return result

for period in (1, 2, 5, 14, 20, 50):
    sma = pd.Series(close).rolling(period, min_periods=period).mean().to_numpy()
    ema = np.full(len(close), np.nan)
    if len(close) >= period:
        ema[period - 1] = np.mean(close[:period])
        alpha = 2 / (period + 1)
        for i in range(period, len(close)):
            ema[i] = alpha * close[i] + (1 - alpha) * ema[i - 1]
    ag, al = wilders(gain, period), wilders(loss, period)
    with np.errstate(divide="ignore", invalid="ignore"):
        rsi = 100 - 100 / (1 + ag / al)
    rsi[(al == 0) & (ag > 0)] = 100
    rsi[(ag == 0) & (al > 0)] = 0
    rsi[(ag == 0) & (al == 0)] = 50
    atr = wilders(tr, period)
    mid = sma
    sd = np.full(len(close), np.nan)
    sd[period - 1:] = np.std(
        np.lib.stride_tricks.sliding_window_view(close, period), axis=1, ddof=0
    )
    upper, lower = mid + 2 * sd, mid - 2 * sd
    # Input has no volume field. Keep volume zero so the fixture makes the limitation explicit.
    relvol = np.full(len(close), np.nan)
    for kind, values in (("sma", sma), ("ema", ema), ("rsi", rsi), ("atr", atr),
                         ("bb_mid", mid), ("bb_upper", upper), ("bb_lower", lower),
                         ("rel_vol", relvol)):
        pd.DataFrame({"value": values}).to_csv(out / f"{kind}_{period}.csv", index=False, float_format="%.17g", na_rep="None")

vwap = np.full(len(close), np.nan)
pd.DataFrame({"value": vwap}).to_csv(out / "vwap.csv", index=False, float_format="%.17g", na_rep="None")
