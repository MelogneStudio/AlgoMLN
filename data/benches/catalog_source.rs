use algomln::{
    sim::{CandleSource, DateRange, Timeframe},
    Symbol,
};
use algomln_data::{Bar, Catalog, CatalogSource, Store, Tf};
use chrono::{Datelike, Duration, NaiveDate, TimeZone};
use chrono_tz::Asia::Kolkata;
use criterion::{black_box, criterion_group, criterion_main, Criterion, Throughput};
use std::{
    alloc::{GlobalAlloc, Layout, System},
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};

struct Meter;
static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);
unsafe impl GlobalAlloc for Meter {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        let p = System.alloc(l);
        if !p.is_null() {
            let n = LIVE.fetch_add(l.size(), Ordering::Relaxed) + l.size();
            PEAK.fetch_max(n, Ordering::Relaxed);
        }
        p
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        System.dealloc(p, l);
        LIVE.fetch_sub(l.size(), Ordering::Relaxed);
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        let q = System.realloc(p, l, n);
        if !q.is_null() {
            let z = if n >= l.size() {
                LIVE.fetch_add(n - l.size(), Ordering::Relaxed) + n - l.size()
            } else {
                LIVE.fetch_sub(l.size() - n, Ordering::Relaxed) - (l.size() - n)
            };
            PEAK.fetch_max(z, Ordering::Relaxed);
        }
        q
    }
}
#[global_allocator]
static ALLOCATOR: Meter = Meter;

fn prepare(root: &PathBuf, symbols: &[String], dates: &[NaiveDate]) {
    let db = root.join("catalog.sqlite");
    let mut cat = Catalog::open(db).unwrap();
    let scrips: Vec<_> = symbols
        .iter()
        .enumerate()
        .map(|(i, s)| dhan_api::Scrip {
            security_id: (i + 1).to_string(),
            symbol: s.clone(),
            lot_size: 1,
            tick_size_paise: 5,
            instrument: "EQUITY".into(),
        })
        .collect();
    cat.refresh_instruments(&scrips, 0).unwrap();
    let store = Store::new(root);
    for (i, _s) in symbols.iter().enumerate() {
        let mut bars = Vec::with_capacity(dates.len() * 360);
        for d in dates {
            let open = d
                .and_hms_opt(9, 15, 0)
                .unwrap()
                .and_local_timezone(Kolkata)
                .single()
                .unwrap()
                .timestamp();
            for n in 0..360 {
                let p = 100.0 + i as f64 + n as f64 / 1000.0;
                bars.push(Bar {
                    ts: open + n * 60,
                    open: p,
                    high: p + 0.01,
                    low: p - 0.01,
                    close: p,
                    volume: 100.0,
                });
            }
        }
        let parts = store
            .write_partitions(&(i + 1).to_string(), Tf::M1, &bars)
            .unwrap();
        let sid = cat.series_for_import(&(i + 1).to_string(), Tf::M1).unwrap();
        for p in &parts {
            cat.record_partition(sid, &store, p).unwrap();
        }
    }
}
fn bench(c: &mut Criterion) {
    let symbols: Vec<String> = (0..100).map(|n| format!("S{n:03}")).collect();
    let mut dates = Vec::new();
    let mut d = NaiveDate::from_ymd_opt(2021, 10, 1).unwrap();
    let end = NaiveDate::from_ymd_opt(2026, 10, 1).unwrap();
    while d <= end {
        if d.weekday().number_from_monday() <= 5 {
            dates.push(d);
        }
        d += Duration::days(1);
    }
    let root = std::env::temp_dir().join(format!("algomln-catalog-bench-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    eprintln!(
        "preparing ~{}M synthetic candles; loader is timed separately",
        symbols.len() * dates.len() * 360 / 1_000_000
    );
    prepare(&root, &symbols, &dates);
    let source = CatalogSource::new(Store::new(&root), root.join("catalog.sqlite"));
    let syms: Vec<Symbol> = symbols.iter().map(|s| s.as_str().into()).collect();
    let range = DateRange {
        from: Some(
            Kolkata
                .from_local_datetime(&dates[0].and_hms_opt(0, 0, 0).unwrap())
                .single()
                .unwrap()
                .timestamp(),
        ),
        to: Some(
            Kolkata
                .from_local_datetime(&dates[dates.len() - 1].and_hms_opt(23, 59, 59).unwrap())
                .single()
                .unwrap()
                .timestamp(),
        ),
    };
    let count = symbols.len() * dates.len() * 360;
    let mut group = c.benchmark_group("catalog_source_loader_only");
    group.throughput(Throughput::Elements(count as u64));
    group.bench_function("100_symbols_5y_1m", |b| {
        b.iter(|| {
            let before = PEAK.load(Ordering::Relaxed);
            let stream = source.open(&syms, Timeframe::M1, range).unwrap();
            let n = stream.map(Result::unwrap).count();
            black_box(n);
            let peak = PEAK.load(Ordering::Relaxed).saturating_sub(before);
            eprintln!(
                "catalog source peak allocation delta: {:.1} MiB",
                peak as f64 / 1024.0 / 1024.0
            );
        })
    });
    group.finish();
    let _ = std::fs::remove_dir_all(root);
}
criterion_group!(benches, bench);
criterion_main!(benches);
