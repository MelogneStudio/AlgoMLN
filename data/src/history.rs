use crate::error::DataError;
use crate::types::Bar;
use chrono::NaiveDate;
use dhan_api::DhanClient;

pub trait HistoryApi: Send + Sync {
    fn intraday(
        &self,
        id: &str,
        instrument: &str,
        interval: &str,
        from: NaiveDate,
        to: NaiveDate,
    ) -> Result<Vec<Bar>, DataError>;
    fn daily(
        &self,
        id: &str,
        instrument: &str,
        from: NaiveDate,
        to: NaiveDate,
    ) -> Result<Vec<Bar>, DataError>;
}

impl HistoryApi for DhanClient {
    fn intraday(
        &self,
        id: &str,
        instrument: &str,
        interval: &str,
        from: NaiveDate,
        to: NaiveDate,
    ) -> Result<Vec<Bar>, DataError> {
        Ok(
            DhanClient::intraday(self, id, instrument, interval, from, to)?
                .into_iter()
                .map(Into::into)
                .collect(),
        )
    }

    fn daily(
        &self,
        id: &str,
        instrument: &str,
        from: NaiveDate,
        to: NaiveDate,
    ) -> Result<Vec<Bar>, DataError> {
        Ok(DhanClient::daily(self, id, instrument, from, to)?
            .into_iter()
            .map(Into::into)
            .collect())
    }
}
