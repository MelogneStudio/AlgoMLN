pub mod candle;
pub mod order;
pub mod position;
pub mod quote;
pub mod tick;

pub use candle::Candle;
pub use order::{Order, OrderResult, OrderSide, OrderStatus, OrderType};
pub use position::{Portfolio, Position};
pub use quote::Quote;
pub use tick::Tick;

// Re-export for plugin API surface — the plugin events module types
// `crate::models::PaperTrade` so the engine trade type is reachable
// from outside the strategy module.
pub use crate::strategy::execution::paper::PaperTrade;
