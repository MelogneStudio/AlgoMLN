use std::collections::HashMap;
use std::sync::Arc;

use crate::commands::registry::StrategyMode;
use crate::models::Candle;
use crate::plugin::api::events::EventBus;
use crate::strategy::execution::paper::PaperBroker;
use crate::strategy::logging::log::LogEntry;
use crate::strategy::{
    dsl::ast::{StrategyNode, TradeIn},
    runtime::engine::{StrategyEngine, StrategyInstance, StrategyStatus},
};

/// Resolves a `TradeIn` clause to a concrete list of NSE symbols using the
/// shared `IndexRegistry`. Returns `Err` for the `Index` variant if the alias
/// has no symbols loaded — the user needs to refresh from Settings.
pub fn resolve_trade_in_symbols(
    trade_in: &TradeIn,
    index_registry: &crate::indices::IndexRegistry,
) -> Result<Vec<String>, String> {
    match trade_in {
        TradeIn::Symbols(symbols) => {
            if symbols.is_empty() {
                return Err("TRADE_IN symbol list is empty".to_string());
            }
            Ok(symbols.iter().map(|s| s.to_uppercase()).collect())
        }
        TradeIn::Index(alias) => {
            let symbols = index_registry.get_symbols(alias);
            if symbols.is_empty() {
                Err(format!(
                    "Index '{}' has no symbols loaded. Open Settings → Index Data and click Refresh.",
                    alias.display_name()
                ))
            } else {
                Ok(symbols)
            }
        }
    }
}

/// Runs N independent `StrategyEngine` instances — one per symbol — against a single
/// shared `PaperBroker`. Capital is shared (positions across all symbols sum up).
/// Each sub-engine stamps orders with its own symbol; the `PaperBroker` tracks them
/// separately in its existing `HashMap<String, PaperPosition>`.
///
/// Invariant: sub-engines are never accessed concurrently. `on_tick` takes `&mut self`
/// and dispatches to exactly one sub-engine per call.
pub struct PortfolioEngine {
    /// Symbol (uppercase) → StrategyEngine
    sub_engines: HashMap<String, StrategyEngine>,
    /// Shared broker — held here for direct position/PnL inspection
    broker: Arc<PaperBroker>,
    /// Symbols in insertion order (deterministic for logging)
    symbol_order: Vec<String>,
}

impl PortfolioEngine {
    /// Create a portfolio engine for the given parsed strategy and symbol list.
    ///
    /// `strategy` must NOT have `trade_in` validated here — that is the caller's job.
    /// `symbols` must be non-empty.
    /// `initial_cash` is the shared paper capital across all symbols.
    /// `event_bus` is `None` for deterministic runs; `Some(bus)` for paper/live runs.
    pub fn new(
        strategy: &StrategyNode,
        symbols: Vec<String>,
        initial_cash: f64,
        event_bus: Option<Arc<EventBus>>,
    ) -> Self {
        assert!(
            !symbols.is_empty(),
            "PortfolioEngine: symbols must be non-empty"
        );

        // The shared broker doesn't know about any single symbol — its inner
        // `positions: HashMap<String, PaperPosition>` is keyed by whatever
        // symbol the sub-engine stamps onto each order. We seed it with the
        // first symbol for `PaperBroker::symbol` bookkeeping (used only by
        // tests / debug prints) but the per-symbol positions are tracked
        // independently.
        let broker: Arc<PaperBroker> = Arc::new(PaperBroker::new(
            symbols.first().cloned().unwrap_or_default(),
            initial_cash,
        ));
        let mut sub_engines: HashMap<String, StrategyEngine> = HashMap::new();
        let mut symbol_order: Vec<String> = Vec::with_capacity(symbols.len());

        for symbol in &symbols {
            let sym_upper = symbol.to_uppercase();
            let instance = StrategyInstance {
                id: format!("portfolio-{}", sym_upper),
                strategy: std::sync::Arc::new(strategy.clone()),
                symbol: sym_upper.clone(),
                timeframe: crate::broker::Timeframe::M5,
                status: StrategyStatus::Running,
                execution_target: Arc::clone(&broker)
                    as Arc<dyn crate::strategy::execution::target::ExecutionTarget>,
            };

            let mut engine = StrategyEngine::new(instance);
            engine.event_bus = event_bus.clone();
            sub_engines.insert(sym_upper.clone(), engine);
            symbol_order.push(sym_upper);
        }

        Self {
            sub_engines,
            broker,
            symbol_order,
        }
    }

    /// Convenience constructor that resolves a `TradeIn` clause into a symbol
    /// list using the given `IndexRegistry`, then calls `new`.
    pub fn from_trade_in(
        strategy: &StrategyNode,
        trade_in: &TradeIn,
        index_registry: &crate::indices::IndexRegistry,
        initial_cash: f64,
        event_bus: Option<Arc<EventBus>>,
    ) -> Result<Self, String> {
        let symbols = resolve_trade_in_symbols(trade_in, index_registry)?;
        Ok(Self::new(strategy, symbols, initial_cash, event_bus))
    }

    /// Advance one symbol's engine by one candle. Returns that engine's log entries.
    /// If `symbol` is unknown, logs a warning to stderr and returns empty vec.
    pub async fn on_tick(&mut self, symbol: &str, candles: &[Candle]) -> Vec<LogEntry> {
        let key = symbol.to_uppercase();
        match self.sub_engines.get_mut(&key) {
            Some(engine) => engine.on_candle(candles).await,
            None => {
                eprintln!(
                    "[PortfolioEngine] on_tick called for unregistered symbol '{}' — ignoring",
                    symbol
                );
                Vec::new()
            }
        }
    }

    /// Symbols this engine is tracking, in stable insertion order.
    pub fn symbols(&self) -> &[String] {
        &self.symbol_order
    }

    /// Direct access to the shared broker for position/PnL snapshots.
    pub fn broker(&self) -> &Arc<PaperBroker> {
        &self.broker
    }

    pub fn symbol_count(&self) -> usize {
        self.sub_engines.len()
    }

    /// Direct access to a sub-engine (e.g. for status flips). The caller MUST
    /// not call `on_candle` on the sub-engine while `PortfolioEngine::on_tick`
    /// holds `&mut self` — see CLAUDE.md invariant 11.
    pub fn sub_engine(&self, symbol: &str) -> Option<&StrategyEngine> {
        self.sub_engines.get(&symbol.to_uppercase())
    }

    /// Mutable sub-engine access — same constraint as `sub_engine`.
    pub fn sub_engine_mut(&mut self, symbol: &str) -> Option<&mut StrategyEngine> {
        self.sub_engines.get_mut(&symbol.to_uppercase())
    }

    /// Construct a sub-engine for a given symbol with a caller-supplied mode
    /// instead of the default `Paper`. Currently only `Paper` is supported
    /// end-to-end (shared `Arc<PaperBroker>`); the `mode` argument is stored
    /// for forward compatibility and is currently unused.
    #[allow(unused_variables)]
    pub fn new_with_mode(
        strategy: &StrategyNode,
        symbols: Vec<String>,
        initial_cash: f64,
        mode: StrategyMode,
        event_bus: Option<Arc<EventBus>>,
    ) -> Self {
        // The portfolio engine currently only brokers through a shared
        // PaperBroker; live-broker wiring is a future prompt. The mode is
        // accepted so callers can express intent without changing the API.
        Self::new(strategy, symbols, initial_cash, event_bus)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::indices::IndexRegistry;
    use crate::strategy::dsl::ast::IndexAlias;
    use crate::strategy::dsl::{AstValidator, Lexer, Parser};
    use crate::strategy::logging::log::LogEntryKind;

    fn parse(source: &str) -> StrategyNode {
        let tokens = Lexer::tokenize(source).expect("lex");
        let node = Parser::new(tokens).parse().expect("parse");
        let errors = AstValidator::validate(&node);
        assert!(errors.is_empty(), "validate: {errors:?}");
        node
    }

    #[test]
    fn resolves_explicit_symbols_to_uppercase() {
        let registry = IndexRegistry::new();
        let trade_in = TradeIn::Symbols(vec!["reliance".into(), "infy".into()]);
        let syms = resolve_trade_in_symbols(&trade_in, &registry).unwrap();
        assert_eq!(syms, vec!["RELIANCE", "INFY"]);
    }

    #[test]
    fn rejects_empty_explicit_symbols() {
        let registry = IndexRegistry::new();
        let trade_in = TradeIn::Symbols(vec![]);
        let err = resolve_trade_in_symbols(&trade_in, &registry).unwrap_err();
        assert!(err.contains("empty"));
    }

    #[test]
    fn index_with_no_symbols_returns_error() {
        let registry = IndexRegistry::new();
        let trade_in = TradeIn::Index(IndexAlias::NiftyBank);
        let err = resolve_trade_in_symbols(&trade_in, &registry).unwrap_err();
        assert!(err.contains("NIFTY BANK"));
    }

    #[test]
    fn index_with_loaded_symbols_returns_them() {
        let registry = IndexRegistry::new();
        registry.update(
            IndexAlias::NiftyBank,
            vec!["HDFCBANK".into(), "ICICIBANK".into()],
            "2026-07-17".into(),
        );
        let trade_in = TradeIn::Index(IndexAlias::NiftyBank);
        let syms = resolve_trade_in_symbols(&trade_in, &registry).unwrap();
        assert_eq!(syms, vec!["HDFCBANK", "ICICIBANK"]);
    }

    #[test]
    fn new_creates_sub_engines_per_symbol() {
        let strategy = parse("WHEN rsi(14) < 30\nBUY 1");
        let engine = PortfolioEngine::new(
            &strategy,
            vec!["RELIANCE".into(), "INFY".into()],
            100_000.0,
            None,
        );
        assert_eq!(engine.symbol_count(), 2);
        assert_eq!(engine.symbols(), &["RELIANCE", "INFY"]);
    }

    #[test]
    fn new_with_empty_symbols_panics() {
        let strategy = parse("WHEN rsi(14) < 30\nBUY 1");
        let result = std::panic::catch_unwind(|| {
            PortfolioEngine::new(&strategy, vec![], 100_000.0, None);
        });
        assert!(result.is_err());
    }

    fn candle(close: f64) -> Candle {
        Candle {
            timestamp: close as i64,
            open: close,
            high: close,
            low: close,
            close,
            volume: 1000.0,
        }
    }

    #[tokio::test]
    async fn on_tick_for_unknown_symbol_returns_empty() {
        let strategy = parse("WHEN close > 0\nBUY 1");
        let mut engine = PortfolioEngine::new(&strategy, vec!["RELIANCE".into()], 100_000.0, None);
        let logs = engine.on_tick("UNKNOWN", &[candle(1.0)]).await;
        assert!(logs.is_empty());
    }

    #[tokio::test]
    async fn sub_engines_share_broker_cash() {
        let strategy = parse("WHEN close > 0\nBUY 1");
        let mut engine =
            PortfolioEngine::new(&strategy, vec!["AAA".into(), "BBB".into()], 10_000.0, None);

        // Drive each sub-engine through one candle. `close > 0` fires the rule
        // once per sub-engine (trigger state prevents re-fire).
        let logs_a = engine.on_tick("AAA", &[candle(1.0)]).await;
        let logs_b = engine.on_tick("BBB", &[candle(1.0)]).await;

        // One trade per sub-engine → two trades total.
        let trades_a = logs_a
            .iter()
            .filter(|e| matches!(e.kind, LogEntryKind::OrderExecuted { .. }))
            .count();
        let trades_b = logs_b
            .iter()
            .filter(|e| matches!(e.kind, LogEntryKind::OrderExecuted { .. }))
            .count();
        assert_eq!(trades_a, 1);
        assert_eq!(trades_b, 1);

        // Both positions live in the same broker.
        let state = engine.broker().get_state();
        assert_eq!(state.positions.len(), 2);
        let symbols: std::collections::BTreeSet<_> =
            state.positions.iter().map(|p| p.symbol.clone()).collect();
        assert!(symbols.contains("AAA"));
        assert!(symbols.contains("BBB"));
    }
}
