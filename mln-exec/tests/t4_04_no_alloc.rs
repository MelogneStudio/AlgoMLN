use std::alloc::{GlobalAlloc, Layout, System};
use std::hint::black_box;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use mln_exec::{Bar, Executor};
use mln_ir::lower_default;
use mln_lang::parse;

struct CountingAlloc;

static COUNTING: AtomicBool = AtomicBool::new(false);
static ALLOC_CALLS: AtomicUsize = AtomicUsize::new(0);
static REALLOC_CALLS: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if COUNTING.load(Ordering::SeqCst) {
            ALLOC_CALLS.fetch_add(1, Ordering::SeqCst);
        }
        System.alloc(layout)
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        if COUNTING.load(Ordering::SeqCst) {
            ALLOC_CALLS.fetch_add(1, Ordering::SeqCst);
        }
        System.alloc_zeroed(layout)
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        System.dealloc(ptr, layout)
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if COUNTING.load(Ordering::SeqCst) {
            REALLOC_CALLS.fetch_add(1, Ordering::SeqCst);
        }
        System.realloc(ptr, layout, new_size)
    }
}

#[global_allocator]
static GLOBAL: CountingAlloc = CountingAlloc;

fn make_bars(len: usize) -> Vec<Bar> {
    let mut bars = Vec::with_capacity(len);
    let mut price = 100.0_f64;
    let mut ts = 1_700_000_000_i64;
    let mut rng = 0x1234_5678_9abc_def0_u64;
    for i in 0..len {
        rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1);
        let delta = ((rng >> 32) as i32 % 150) as f64 * 0.01;
        let open = price;
        let close = (price + delta).max(1.0);
        price = close;
        let high = open.max(close) + 0.5 + (i % 4) as f64 * 0.1;
        let low = (open.min(close) - 0.5).max(0.5);
        let volume = 500.0 + (i % 1000) as f64;
        ts += 60;
        bars.push(Bar::new(ts, open, high, low, close, volume));
    }
    bars
}

#[test]
fn t4_04_zero_heap_allocations_after_warmup() {
    let src = "\
def spread(x, y) = (x - y) / max(abs(y), 1.0)
s = sma(close, 20)
e = ema(close, 20)
r = rma(close, 14)
w = wma(close, 15)
sv = sum(volume, 10)
sd = std(close, 20)
vr = var(close, 20)
hi = highest(high, 20)
lo = lowest(low, 20)
rs = rsi(close, 14)
trv = tr(high, low, close)
atrv = atr(high, low, close, 14)
mc = macd(close, 12, 26, 9)
bb = bollinger(close, 20, 2.0)
co = corr(close, volume, 20)
cv = cov(high, low, 20)
lrs = linreg_slope(close, 20)
lri = linreg_intercept(close, 20)
pr = percentile_rank(close, 25)
cs = cumsum(diff(close, 1))
pc = pct_change(close, 3)
lag = close[5]
sp = spread(s, e)
cond = crossover(s, e) or (rs > 50.0 and mc > -1e9 and bb > -1e9 and sp == sp)
picked = where(cond, hi, lo)
entry = true if (cond and atrv > 0.0 and sd >= 0.0 and vr >= 0.0 and co == co and cv == cv and lrs == lrs and lri == lri and pr >= 0.0 and cs == cs and pc == pc and picked > lag and trv >= 0.0 and sv > 0.0 and r > 0.0 and w > 0.0) else false
exit = crossunder(s, e) or since(s > e) > 10
";
    let parsed = parse(src);
    assert!(parsed.diags.is_empty(), "parse failed: {:?}", parsed.diags);
    let ir = lower_default(&parsed.program).expect("lower failed");

    let mut exec = Executor::new(&ir).expect("executor init");
    let warmup_bars = make_bars(100);
    let live_bars = make_bars(5_000);
    let mut chunk_out = Vec::with_capacity(500);

    // Warm up every rolling/stateful operator and the reusable output buffer.
    for &b in &warmup_bars {
        black_box(exec.step(b).expect("warmup step"));
    }

    ALLOC_CALLS.store(0, Ordering::SeqCst);
    REALLOC_CALLS.store(0, Ordering::SeqCst);
    COUNTING.store(true, Ordering::SeqCst);

    for &b in &live_bars {
        let out = exec.step(b).expect("live step");
        black_box(out);
    }

    for chunk in live_bars.chunks(500) {
        chunk_out.clear();
        exec.run_chunk_into(chunk, &mut chunk_out)
            .expect("live chunk");
        black_box(chunk_out.as_slice());
    }

    COUNTING.store(false, Ordering::SeqCst);

    let allocs = ALLOC_CALLS.load(Ordering::SeqCst);
    let reallocs = REALLOC_CALLS.load(Ordering::SeqCst);
    assert_eq!(
        allocs, 0,
        "expected 0 heap allocations in step/run_chunk_into, saw {allocs}"
    );
    assert_eq!(
        reallocs, 0,
        "expected 0 heap reallocs in step/run_chunk_into, saw {reallocs}"
    );
}
