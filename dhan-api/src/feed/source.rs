use crate::feed::normalize::Tick;

#[derive(Clone, Debug, PartialEq)]
pub enum FeedEvent {
    Tick(Tick),
    PrevClose {
        security_id: i32,
        price_paise: i64,
        oi: i32,
    },
    Connected,
    Disconnected {
        reason: Option<i16>,
    },
    Stale {
        security_id: i32,
    },
}

pub trait TickSource {
    fn next(&mut self) -> Option<FeedEvent>;

    /// Returns current feed time in UTC epoch seconds, if available.
    /// In replay, this comes from the recorded frame time 't'.
    /// In live, this comes from the system clock.
    fn current_time_utc(&self) -> Option<i64> {
        None
    }
}
