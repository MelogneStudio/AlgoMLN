use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;

use parking_lot::Mutex;
use tokio_util::sync::CancellationToken;

use crate::plugin::types::{PluginError, PluginResult, ScheduleHandle};

use super::SchedulerApi;

pub struct CronScheduler {
    handles: Arc<Mutex<HashMap<ScheduleHandle, CancellationToken>>>,
}

impl CronScheduler {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            handles: Arc::new(Mutex::new(HashMap::new())),
        })
    }
}

#[async_trait::async_trait]
impl SchedulerApi for CronScheduler {
    fn schedule(
        &self,
        cron_expr: &str,
        task: Arc<dyn Fn() + Send + Sync>,
    ) -> PluginResult<ScheduleHandle> {
        let schedule = cron::Schedule::from_str(cron_expr)
            .map_err(|e| PluginError::ApiError(format!("invalid cron expression: {e}")))?;
        let handle = ScheduleHandle(uuid::Uuid::new_v4());
        let token = CancellationToken::new();

        let task_token = token.clone();
        tokio::spawn(async move {
            loop {
                let now = chrono::Utc::now();
                let next = match schedule.upcoming(chrono::Utc).next() {
                    Some(t) => t,
                    None => break,
                };
                if next <= now {
                    // Skip past times.
                    continue;
                }
                let sleep_until = tokio::time::Instant::now()
                    + (next - now).to_std().unwrap_or(std::time::Duration::from_secs(0));
                tokio::select! {
                    _ = tokio::time::sleep_until(sleep_until) => {
                        task();
                    }
                    _ = task_token.cancelled() => {
                        break;
                    }
                }
            }
        });

        self.handles.lock().insert(handle, token);
        Ok(handle)
    }

    fn cancel(&self, handle: ScheduleHandle) -> PluginResult<()> {
        let token = self
            .handles
            .lock()
            .remove(&handle)
            .ok_or_else(|| PluginError::NotFound("schedule not found".into()))?;
        token.cancel();
        Ok(())
    }
}
