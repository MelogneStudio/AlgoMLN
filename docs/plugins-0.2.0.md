# AlgoMLN Plugin System Guide

Extend the AlgoMLN trading platform without modifying the core engine. Plugins allow you to inject custom logic into the strategy evaluation pipeline, create custom UI components, and automate tasks.

## 📌 Overview

Plugins are sandboxed, capability-gated, and hot-reloadable. They live in `<app_data>/plugins/<plugin-id>/` and are discovered at startup.

### Key Architecture Invariants
- **Stateless Callbacks**: To prevent memory leaks and reference cycles, Rhai plugins must use **named functions** for callbacks. Closures (capturing functions) are strictly forbidden and will be rejected at registration.
- **Capability Gating**: Every platform service (Storage, Execution, etc.) is gated. If a plugin calls a function it hasn't declared in its manifest, the call returns `PermissionDenied`. Logging is the only always-available capability.
- **Isolation**: Plugins are isolated from each other. Storage is namespaced by plugin ID, and the engine ensures that one plugin cannot interfere with another's state.

---

## 📄 The Manifest (`plugin.toml`)

Every plugin requires a `plugin.toml` file in its root directory. This file defines the plugin's identity and its permission set.

```toml
# plugin.toml
id      = "my-custom-plugin"
name    = "My Custom Plugin"
version = "1.0.0"
entry   = "plugin.rhai"       # Extension determines runtime (.rhai or .wasm)

[permissions]
capabilities = [
    "Indicators",             # Register custom indicators for .algomln strategies
    "Analytics",              # Add custom metrics to backtest results
    "DslExtension",           # Add new keywords to the DSL
    "Storage",                # Access per-plugin key-value store
    "UiPanels",               # Create custom UI panels and notifications
    "Scheduler",              # Schedule recurring cron tasks
    "MarketData",             # Read latest candles/ticks
    "Execution",              # Submit orders (Gated by live session)
]
```

### Manifest Fields
| Field | Type | Description |
| :--- | :--- | :--- |
| `id` | String | Unique identifier. Used for storage keys and deduping. |
| `name` | String | Human-readable name displayed in the Plugins screen. |
| `version` | String | Semver version of the plugin. |
| `entry` | String | The main script/binary file. `.rhai` $\to$ Rhai, `.wasm` $\to$ WASM. |
| `capabilities` | String[] | List of platform services the plugin is allowed to use. |

---

## ⚙️ Runtimes

### Rhai Runtime (`.rhai`)
Rhai is a lightweight, Rust-like scripting language. It is the primary way to write plugins due to its support for complex callback registries.

**Containment Budgets:**
- **Operation Limit**: Maximum AST operations per call to prevent infinite loops.
- **Recursion Limit**: Preves stack overflows.
- **Collection Limits**: Caps array and map sizes.
- **No Imports**: Module loading is disabled for security.

### WASM Runtime (`.wasm`)
WASM plugins provide near-native performance and are compiled from languages like Rust or C++.

**Current Status:**
- **Non-Callback APIs Only**: WASM plugins can use most host functions (Storage, UI, Execution, etc.) but **cannot** currently register callbacks (Indicators, Metrics, Keywords, Schedules, Event subscriptions).
- **Strict Isolation**: No WASI access; only the `algomln::*` host surface is exposed.
- **CPU Budget**: A watchdog timer kills any execution that exceeds the CPU budget.

---

## 🛠️ The Rhai API Reference

Rhai plugins interact with the platform through global host functions.

### 1. Lifecycle Hooks
Implement these functions in your script to handle plugin state transitions.

| Hook | When Called | Use Case |
| :--- | :--- | :--- |
| `on_load()` | At startup / scan | Register indicators, keywords, and metrics. |
| `on_enable()` | User enables plugin | Setup event subscriptions and cron tasks. |
| `on_disable()` | User disables plugin | Clean up temporary resources. |
| `on_unload()` | App shutdown / reload | Final cleanup. |

### 2. Callback Registration (Stateless Architecture)
To register a callback, you must define a **named function** and pass its name to the registration function.

#### Custom Indicators
Register a function that calculates values for a series of candles.
```rhai
fn my_custom_indicator(candles, period) {
    // candles: Array of Candle objects
    // period: Integer
    // Return: Array of floats (one per candle)
    [1.0, 2.0, 3.0] 
}

fn on_load() {
    register_indicator("my_ind", my_custom_indicator);
}
```
*Use in `.algomln`:* `WHEN my_ind(10) > 5 BUY 1`

#### Custom Metrics (Analytics)
Compute a value over a list of completed trades.
```rhai
fn calculate_profit_factor(trades) {
    // trades: Array of Trade objects
    // Return: float
    1.5
}

fn on_load() {
    register_metric("profit_factor", calculate_profit_factor);
}
```

#### DSL Keywords
Extend the strategy language with new boolean conditions.
```rhai
fn is_volatility_high(candles, current) {
    // current: The current candle
    current.high - current.low > 10.0
}

fn on_load() {
    register_keyword("high_vol", is_volatility_high);
}
```
*Use in `.algomln`:* `WHEN high_vol AND close > open BUY 1`

#### Scheduled Tasks
Run logic on a cron schedule. **Tasks are auto-disabled after 3 consecutive failures.**
```rhai
fn daily_report() {
    log_info("Running daily health check...");
}

fn on_enable() {
    // Fire every day at 09:00
    schedule("0 9 * * *", daily_report);
}
```

#### Event Subscriptions
Listen for engine events.
```rhai
fn on_trade_executed(event) {
    log_info("Trade executed: " + event.trade.id);
}

fn on_enable() {
    subscribe_event("TradeExecuted", on_trade_executed);
}
```

### 3. General Host Functions

| Function | Arguments | Description |
| :--- | :--- | :--- |
| `log_info(msg)` | `String` | Logs a message to the plugin log file. |
| `log_warn(msg)` | `String` | Logs a warning. |
| `log_error(msg)` | `String` | Logs an error. |
| `storage_get(key)` | `String` $\to$ `String` | Retrieves a value from the per-plugin KV store. |
| `storage_set(key, val)`| `String, String` $\to$ `bool` | Saves a value to the KV store. |
| `storage_delete(key)` | `String` $\to$ `bool` | Removes a key. |
| `notify_info(msg)` | `String` | Pushes an info toast to the UI. |
| `register_panel(id, title)`| `String, String` $\to$ `bool` | Registers a new UI panel. |
| `emit_panel_data(id, json)`| `String, String` $\to$ `bool` | Sends JSON data to a specific UI panel. |
| `submit_order(sym, side, qty, type, price)` | `...` $\to$ `String` | Submits an order (Live only, gated by session). |
| `get_latest_candle(sym)` | `String` $\to$ `Candle` | Fetches the most recent candle for a symbol. |

---

## 🚀 Creating Your First Plugin (Step-by-Step)

1. **Create Folder**: Navigate to your app data folder and create `plugins/my-first-plugin/`.
2. **Create Manifest**: Create `plugin.toml` with the required ID and capabilities.
3. **Write Script**: Create `plugin.rhai`.
   ```rhai
   fn my_metric(trades) {
       trades.len() as float
   }
   fn on_load() {
       log_info("My First Plugin Loaded!");
       register_metric("total_trades", my_metric);
   }
   ```
4. **Enable**: Open the AlgoMLN desktop app, go to the **Plugins** screen, and toggle "My First Plugin" to **Enabled**.

## ⚠️ Troubleshooting & Limits

- **Rate Limiting**: Plugin logs are rate-limited (10 msg/sec burst, 100 msg/min sustained). Excess logs are dropped.
- **Rolling Logs**: Log files are capped at 5MB; they rotate automatically.
- **Execution Gating**: `submit_order` will fail if no live session is active or if the market is closed.
- **WASM Limitations**: If you need to register an indicator or a scheduled task, you **must** use Rhai. WASM is for high-performance data processing and general utility.
