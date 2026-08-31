#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::missing_panics_doc,
    missing_docs,
    reason = "This is a temporary benchmark harness."
)]

mod cases;
mod data;
mod fine;

use std::cell::RefCell;

use serde::Serialize;
use wasm_bindgen::prelude::*;

use crate::cases::BenchCase;

thread_local! {
    static CASES: RefCell<Option<Vec<Box<dyn BenchCase>>>> = const { RefCell::new(None) };
}

#[derive(Serialize)]
struct BuildInfo {
    revision: &'static str,
    variant: &'static str,
    simd128: bool,
}

#[derive(Serialize)]
struct BenchmarkResult {
    name: String,
    iterations: u32,
    samples_ns: Vec<f64>,
    median_ns: f64,
    mean_ns: f64,
    min_ns: f64,
    max_ns: f64,
    stddev_ns: f64,
}

#[wasm_bindgen(start)]
pub fn start() {
    console_error_panic_hook::set_once();
}

#[wasm_bindgen]
pub fn build_info() -> String {
    serde_json::to_string(&BuildInfo {
        revision: option_env!("VELLO_BENCH_REV").unwrap_or("unknown"),
        variant: option_env!("VELLO_BENCH_VARIANT").unwrap_or("unknown"),
        simd128: cfg!(target_feature = "simd128"),
    })
    .unwrap()
}

#[wasm_bindgen]
pub fn benchmark_names() -> String {
    with_cases(|cases| {
        serde_json::to_string(&cases.iter().map(|case| case.name()).collect::<Vec<_>>()).unwrap()
    })
}

#[wasm_bindgen]
pub fn run_benchmark(index: usize) -> Result<String, JsValue> {
    with_cases_mut(|cases| {
        let case = cases
            .get_mut(index)
            .ok_or_else(|| JsValue::from_str("benchmark index is out of range"))?;

        let warmup_start = now();
        while now() - warmup_start < 100.0 {
            case.measure(1);
        }

        let measurement_start = now();
        let mut iterations = 0_u32;
        let mut elapsed_ms = 0.0;
        while now() - measurement_start < 500.0 {
            elapsed_ms += case.measure(1);
            iterations += 1;
        }

        let ns_per_iteration = elapsed_ms * 1_000_000.0 / f64::from(iterations);

        serde_json::to_string(&BenchmarkResult {
            name: case.name().to_owned(),
            iterations,
            min_ns: ns_per_iteration,
            max_ns: ns_per_iteration,
            median_ns: ns_per_iteration,
            mean_ns: ns_per_iteration,
            stddev_ns: 0.0,
            samples_ns: vec![ns_per_iteration],
        })
        .map_err(|error| JsValue::from_str(&error.to_string()))
    })
}

fn with_cases<T>(f: impl FnOnce(&[Box<dyn BenchCase>]) -> T) -> T {
    CASES.with(|slot| {
        let mut slot = slot.borrow_mut();
        let cases = slot.get_or_insert_with(cases::core_cases);
        f(cases)
    })
}

fn with_cases_mut<T>(f: impl FnOnce(&mut [Box<dyn BenchCase>]) -> T) -> T {
    CASES.with(|slot| {
        let mut slot = slot.borrow_mut();
        let cases = slot.get_or_insert_with(cases::core_cases);
        f(cases)
    })
}

pub(crate) fn now() -> f64 {
    web_sys::window().unwrap().performance().unwrap().now()
}

fn main() {}
