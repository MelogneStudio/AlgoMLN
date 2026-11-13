use crate::catalog::Catalog;
use crate::error::DataError;
use crate::types::{Bar, PartitionMeta, Tf};
use arrow_array::{Array, Float64Array, Int64Array, RecordBatch};
use arrow_schema::{DataType, Field, Schema};
use chrono::{DateTime, Utc};
use chrono_tz::Asia::Kolkata;
use parquet::{
    arrow::{arrow_reader::ParquetRecordBatchReaderBuilder, ArrowWriter},
    basic::Compression,
    file::properties::WriterProperties,
};
use rusqlite::params;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashMap},
    fs::{self, File},
    path::{Path, PathBuf},
    sync::Arc,
};

pub struct Store {
    pub root: PathBuf,
}

impl Store {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    fn partition(&self, id: &str, tf: Tf, ts: i64) -> PathBuf {
        let dt = DateTime::<Utc>::from_timestamp(ts, 0)
            .unwrap_or(DateTime::UNIX_EPOCH)
            .with_timezone(&Kolkata);
        let period = if tf == Tf::D1 {
            dt.format("%Y").to_string()
        } else {
            dt.format("%Y-%m").to_string()
        };
        self.root
            .join("candles")
            .join(id)
            .join(tf.as_str())
            .join(format!("{period}.parquet"))
    }

    pub fn write_partitions(
        &self,
        id: &str,
        tf: Tf,
        bars: &[Bar],
    ) -> Result<Vec<PartitionMeta>, DataError> {
        let mut groups: BTreeMap<PathBuf, Vec<Bar>> = BTreeMap::new();
        for b in bars {
            groups
                .entry(self.partition(id, tf, b.ts))
                .or_default()
                .push(b.clone());
        }
        let mut out = Vec::new();
        for (path, chunk) in groups {
            let (sum, first, last, n) = self.write_one(&path, &chunk)?;
            out.push(PartitionMeta {
                path,
                checksum: sum,
                first_ts: first,
                last_ts: last,
                rows: n,
            });
        }
        Ok(out)
    }

    fn write_one(&self, path: &Path, bars: &[Bar]) -> Result<(String, i64, i64, usize), DataError> {
        if let Some(p) = path.parent() {
            fs::create_dir_all(p)?;
        }
        let existing = if path.exists() {
            read_bars(path)?
        } else {
            Vec::new()
        };
        let existing_map: HashMap<i64, &Bar> = existing.iter().map(|b| (b.ts, b)).collect();
        if !existing.is_empty()
            && bars
                .iter()
                .all(|b| existing_map.get(&b.ts).is_some_and(|old| *old == b))
        {
            let sum = checksum(path)?;
            return Ok((
                sum,
                existing.first().unwrap().ts,
                existing.last().unwrap().ts,
                existing.len(),
            ));
        }
        let mut map: BTreeMap<i64, Bar> = existing.into_iter().map(|b| (b.ts, b)).collect();
        for b in bars {
            map.insert(b.ts, b.clone());
        }
        let rows: Vec<_> = map.into_values().collect();
        let schema = Arc::new(Schema::new(vec![
            Field::new("ts", DataType::Int64, false),
            Field::new("open", DataType::Float64, false),
            Field::new("high", DataType::Float64, false),
            Field::new("low", DataType::Float64, false),
            Field::new("close", DataType::Float64, false),
            Field::new("volume", DataType::Float64, false),
        ]));
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(Int64Array::from(
                    rows.iter().map(|x| x.ts).collect::<Vec<_>>(),
                )),
                Arc::new(Float64Array::from(
                    rows.iter().map(|x| x.open).collect::<Vec<_>>(),
                )),
                Arc::new(Float64Array::from(
                    rows.iter().map(|x| x.high).collect::<Vec<_>>(),
                )),
                Arc::new(Float64Array::from(
                    rows.iter().map(|x| x.low).collect::<Vec<_>>(),
                )),
                Arc::new(Float64Array::from(
                    rows.iter().map(|x| x.close).collect::<Vec<_>>(),
                )),
                Arc::new(Float64Array::from(
                    rows.iter().map(|x| x.volume).collect::<Vec<_>>(),
                )),
            ],
        )
        .map_err(|e| DataError::Invalid(e.to_string()))?;
        let tmp = path.with_extension(format!(
            "parquet.tmp-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
        ));
        let file = File::create(&tmp)?;
        let sync_file = file.try_clone()?;
        let props = WriterProperties::builder()
            .set_compression(Compression::SNAPPY)
            .build();
        let mut writer = ArrowWriter::try_new(file, schema, Some(props))
            .map_err(|e| DataError::Invalid(e.to_string()))?;
        writer
            .write(&batch)
            .map_err(|e| DataError::Invalid(e.to_string()))?;
        writer
            .close()
            .map_err(|e| DataError::Invalid(e.to_string()))?;
        sync_file.sync_all()?;
        atomic_replace(&tmp, path)?;
        let sum = checksum(path)?;
        Ok((
            sum,
            rows.first().unwrap().ts,
            rows.last().unwrap().ts,
            rows.len(),
        ))
    }

    pub fn reconcile(&self, cat: &mut Catalog) -> Result<(), DataError> {
        let mut files = HashMap::new();
        let base = self.root.join("candles");
        if base.exists() {
            for p in walk(&base)? {
                if p.extension().and_then(|x| x.to_str()) == Some("parquet") {
                    files.insert(p.clone(), checksum(&p)?);
                }
            }
        }
        // IMMEDIATE so the read-before-write (SELECT partitions, then DELETE/UPDATE)
        // takes the write lock up front: under a concurrent bulk run this avoids a
        // stale-snapshot SQLITE_BUSY between workers; serially it is unchanged.
        let tx = cat
            .conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let mut q = tx.prepare("SELECT p.series_id,p.path,p.checksum FROM partitions p")?;
        let entries = q
            .query_map([], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        drop(q);
        let mut affected = std::collections::HashSet::new();
        for (sid, rel, hash) in entries {
            let p = self.root.join(&rel);
            match files.remove(&p) {
                Some(found) if found == hash => {}
                Some(_) | None => {
                    affected.insert(sid);
                    tx.execute(
                        "DELETE FROM partitions WHERE series_id=?1 AND path=?2",
                        params![sid, rel],
                    )?;
                    tx.execute("DELETE FROM coverage WHERE series_id=?1", [sid])?;
                    tx.execute("DELETE FROM sessions WHERE series_id=?1", [sid])?;
                    tx.execute(
                        "UPDATE series SET first_ts=NULL,last_ts=NULL,rows=0 WHERE id=?1",
                        [sid],
                    )?;
                    tx.execute("INSERT INTO quality(series_id,kind,detail) SELECT ?1,'reconcile_missing_or_checksum','partition removed from catalog coverage' WHERE NOT EXISTS(SELECT 1 FROM quality WHERE series_id=?1 AND kind='reconcile_missing_or_checksum')",[sid])?;
                }
            }
        }
        for (orphan, _) in files {
            tx.execute("INSERT INTO quality(kind,detail) SELECT 'reconcile_orphan_file',?1 WHERE NOT EXISTS(SELECT 1 FROM quality WHERE kind='reconcile_orphan_file' AND detail=?1)",[orphan.to_string_lossy().to_string()])?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn remove_series_files(&self, id: &str, tf: Tf) -> Result<(), DataError> {
        let dir = self.root.join("candles").join(id).join(tf.as_str());
        if dir.exists() {
            fs::remove_dir_all(dir)?;
        }
        Ok(())
    }
}

#[cfg(windows)]
pub(crate) fn atomic_replace(from: &Path, to: &Path) -> Result<(), DataError> {
    use std::os::windows::ffi::OsStrExt;
    let src: Vec<u16> = from.as_os_str().encode_wide().chain(Some(0)).collect();
    let dst: Vec<u16> = to.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut attempts = 0;
    loop {
        let ok = unsafe {
            windows_sys::Win32::Storage::FileSystem::MoveFileExW(
                src.as_ptr(),
                dst.as_ptr(),
                windows_sys::Win32::Storage::FileSystem::MOVEFILE_REPLACE_EXISTING
                    | windows_sys::Win32::Storage::FileSystem::MOVEFILE_WRITE_THROUGH,
            )
        };
        if ok != 0 {
            return Ok(());
        }
        let err = std::io::Error::last_os_error();
        let code = err.raw_os_error().unwrap_or(0) as u32;
        // ERROR_ACCESS_DENIED (5) or ERROR_SHARING_VIOLATION (32)
        if (code == 5 || code == 32) && attempts < 10 {
            attempts += 1;
            std::thread::sleep(std::time::Duration::from_millis(15 * attempts as u64));
            continue;
        }
        return Err(err.into());
    }
}

#[cfg(not(windows))]
pub(crate) fn atomic_replace(from: &Path, to: &Path) -> Result<(), DataError> {
    fs::rename(from, to)?;
    Ok(())
}

fn checksum(p: &Path) -> Result<String, DataError> {
    let bytes = fs::read(p)?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

fn walk(root: &Path) -> Result<Vec<PathBuf>, DataError> {
    let mut v = Vec::new();
    for e in fs::read_dir(root)? {
        let p = e?.path();
        if p.is_dir() {
            v.extend(walk(&p)?)
        } else {
            v.push(p)
        }
    }
    Ok(v)
}

pub fn read_bars(path: &Path) -> Result<Vec<Bar>, DataError> {
    let f = File::open(path)?;
    let r = ParquetRecordBatchReaderBuilder::try_new(f)
        .map_err(|e| DataError::Invalid(e.to_string()))?
        .build()
        .map_err(|e| DataError::Invalid(e.to_string()))?;
    let mut v = Vec::new();
    for batch in r {
        let b = batch.map_err(|e| DataError::Invalid(e.to_string()))?;
        let ts = b
            .column(0)
            .as_any()
            .downcast_ref::<Int64Array>()
            .ok_or_else(|| DataError::Invalid("ts schema".into()))?;
        let cols: Vec<_> = (1..6)
            .map(|i| {
                b.column(i)
                    .as_any()
                    .downcast_ref::<Float64Array>()
                    .ok_or_else(|| DataError::Invalid("OHLCV schema".into()))
            })
            .collect::<Result<_, _>>()?;
        for i in 0..b.num_rows() {
            v.push(Bar {
                ts: ts.value(i),
                open: cols[0].value(i),
                high: cols[1].value(i),
                low: cols[2].value(i),
                close: cols[3].value(i),
                volume: cols[4].value(i),
            });
        }
    }
    Ok(v)
}

pub fn read_partition(path: &Path) -> Result<Vec<Bar>, DataError> {
    read_bars(path)
}
