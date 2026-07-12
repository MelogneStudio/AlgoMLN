fn on_load() {
  log_info("Example plugin loaded");
  let count = storage_get("load_count");
  let n = if count == () { 0 } else { parse_int(count) };
  storage_set("load_count", to_string(n + 1));
  log_info("Load count: " + to_string(n + 1));
  register_indicator("double_ema", double_ema);
}

fn double_ema(candles, period) {
  let closes = [];
  for c in candles { closes += c.close; }
  let ema1 = simple_ema(closes, period);
  let ema2 = simple_ema(ema1, period);
  ema2
}

fn simple_ema(values, period) {
  let result = [];
  let k = 2.0 / (period + 1);
  let prev = values[0];
  for v in values {
    prev = v * k + prev * (1.0 - k);
    result += prev;
  }
  result
}

fn on_enable()  { log_info("Example plugin enabled");  }
fn on_disable() { log_info("Example plugin disabled"); }
fn on_unload()  { log_info("Example plugin unloaded"); }
