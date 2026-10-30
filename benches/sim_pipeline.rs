use algomln::{
    compile,
    indicators::Candle,
    sim::{
        run, CandleSource, CandleStream, DateRange, FeeModel, Fingerprint, GapPolicy, RunConfig,
        Timeframe,
    },
    Symbol,
};
use criterion::{black_box, Criterion, Throughput};
use std::{
    alloc::{GlobalAlloc, Layout, System},
    collections::BTreeSet,
    sync::atomic::{AtomicUsize, Ordering},
};

struct TrackingAllocator;
static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);
static MAX_DELTA: AtomicUsize = AtomicUsize::new(0);
unsafe impl GlobalAlloc for TrackingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let p = System.alloc(layout);
        if !p.is_null() {
            let n = LIVE.fetch_add(layout.size(), Ordering::Relaxed) + layout.size();
            PEAK.fetch_max(n, Ordering::Relaxed);
        }
        p
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        System.dealloc(ptr, layout);
        LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let p = System.realloc(ptr, layout, new_size);
        if !p.is_null() {
            let old = layout.size();
            let n = if new_size >= old {
                LIVE.fetch_add(new_size - old, Ordering::Relaxed) + new_size - old
            } else {
                LIVE.fetch_sub(old - new_size, Ordering::Relaxed) - (old - new_size)
            };
            PEAK.fetch_max(n, Ordering::Relaxed);
        }
        p
    }
}
#[global_allocator]
static ALLOCATOR: TrackingAllocator = TrackingAllocator;

struct BenchSource {
    rows: Vec<(Symbol, Candle)>,
}
impl CandleSource for BenchSource {
    fn open(
        &self,
        symbols: &[Symbol],
        _: Timeframe,
        range: DateRange,
    ) -> Result<CandleStream, algomln::sim::SourceError> {
        let symbols: BTreeSet<_> = symbols.iter().collect();
        let rows = self
            .rows
            .iter()
            .filter(|(s, c)| {
                symbols.contains(s)
                    && range.from.map_or(true, |x| c.timestamp >= x)
                    && range.to.map_or(true, |x| c.timestamp <= x)
            })
            .cloned()
            .map(Ok)
            .collect::<Vec<_>>();
        Ok(Box::new(rows.into_iter()))
    }
    fn fingerprint(&self, _: &[Symbol], _: Timeframe, _: DateRange) -> Fingerprint {
        Fingerprint("synthetic-benchmark".into())
    }
}
fn config() -> RunConfig {
    RunConfig {
        timeframe: Timeframe::M1,
        range: DateRange::default(),
        initial_cash: 1_000_000.0,
        warmup_bars: 0,
        latency: 1,
        slippage_bps: 0.0,
        fee_model: FeeModel::default(),
        partial_fills_pct: None,
        gap_policy: GapPolicy::None,
        debug_invariants: false,
    }
}
fn benchmark(c: &mut Criterion, name: &str, symbols: &[&str], bars: usize) {
    let program = compile(&format!(
        "TRADE_IN {}\nWHEN close > 100000\nBUY 1\n",
        symbols.join(",")
    ))
    .unwrap();
    let mut rows = Vec::with_capacity(bars * symbols.len());
    for i in 0..bars {
        for symbol in symbols {
            let p = 100.0 + (i % 31) as f64;
            rows.push((
                (*symbol).to_owned(),
                Candle {
                    timestamp: i as i64 * 60,
                    open: p,
                    high: p + 1.0,
                    low: p - 1.0,
                    close: p,
                    volume: 1000.0,
                },
            ));
        }
    }
    let source = BenchSource { rows };
    let source_bytes = source.rows.capacity() * std::mem::size_of::<(Symbol, Candle)>()
        + source.rows.iter().map(|(s, _)| s.capacity()).sum::<usize>();
    let count = bars * symbols.len();
    let mut group = c.benchmark_group(name);
    group.throughput(Throughput::Elements(count as u64));
    group.bench_function(name, |b| {
        b.iter(|| {
            let baseline = LIVE.load(Ordering::Relaxed);
            PEAK.store(baseline, Ordering::Relaxed);
            black_box(run(&program, &source, config(), None).unwrap());
            let peak = PEAK.load(Ordering::Relaxed).saturating_sub(baseline) + source_bytes;
            MAX_DELTA.fetch_max(peak, Ordering::Relaxed);
        })
    });
    group.finish();
}
fn main() {
    std::env::set_var("ALGOMLN_BENCHMARK", "1");
    let mut c = Criterion::default().configure_from_args();
    benchmark(&mut c, "sim_pipeline_single_symbol_52k", &["ABC"], 52_000);
    benchmark(
        &mut c,
        "sim_pipeline_multi_symbol_3x52k",
        &["AAA", "BBB", "CCC"],
        52_000,
    );
    c.final_summary();
    eprintln!(
        "maximum observed benchmark heap allocation delta: {} bytes",
        MAX_DELTA.load(Ordering::Relaxed)
    );
}
