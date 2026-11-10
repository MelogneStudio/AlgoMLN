pub mod bulk;
pub mod catalog;
pub mod error;
pub mod history;
pub mod session;
pub mod source;
pub mod store;
pub mod types;
pub mod updater;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod acceptance;

// Public re-exports preserving exact crate API
pub use bulk::{BulkConfig, BulkDownloader, BulkReport, Progress};
pub use catalog::Catalog;
pub use error::DataError;
pub use history::HistoryApi;
pub use session::{
    check_overlap, chunk_dates, chunk_dates_limited, complete_intraday, last_complete_bar,
    local_midnight, missing_date_ranges, session_regime_changes, session_shapes, ts_to_ist_date,
};
pub use source::{
    read_partition_verified, CatalogCursor, CatalogSource, PartitionMetaRow, ResolvedInfo,
};
pub use store::{read_bars, read_partition, Store};
pub use types::{
    drop_in_progress, validate_bar, Bar, Coverage, CoverageRange, Instrument, PartitionMeta,
    Quality, QualityRow, SeriesStatus, Tf,
};
pub use updater::{plan_requests, Updater, DEFAULT_BUDGET_CAP};

// Re-exports used by internal test suites via `use super::*;`
#[cfg(test)]
pub(crate) use bulk::{format_eta, group_thousands};
#[cfg(test)]
pub(crate) use chrono::{Datelike, NaiveDate, Utc};
#[cfg(test)]
pub(crate) use chrono_tz::Asia::Kolkata;
#[cfg(test)]
pub(crate) use rusqlite::{params, Connection};
#[cfg(test)]
pub(crate) use std::{
    collections::{BTreeMap, HashMap},
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};
