use std::io::Read as _;

use flate2::read::GzDecoder;
use perf_to_pprof::{ConvertOptions, Stats, convert, parse};
use prost::Message as _;

const FIXTURE: &str = include_str!("fixtures/sample.perf-script");
const NO_PERIOD: &str = include_str!("fixtures/no-period.perf-script");
const MANY_UNKNOWNS: &str = include_str!("fixtures/many-unknowns.perf-script");

#[test]
fn parses_three_samples_then_one_unknown() {
    let samples = parse(FIXTURE).unwrap();
    assert_eq!(samples.len(), 4);

    // First two samples share an identical 3-frame stack.
    assert_eq!(samples[0].stack.len(), 3);
    assert_eq!(samples[0].stack[0].symbol, "hot_function");
    assert_eq!(samples[0].stack[0].dso, "/path/to/main.exe");
    assert_eq!(samples[0].period, 1_000_000);

    assert_eq!(samples[1].stack.len(), 3);
    assert_eq!(samples[1].stack[0].symbol, "hot_function");

    // Third sample is a different stack.
    assert_eq!(samples[2].stack.len(), 2);
    assert_eq!(samples[2].stack[0].symbol, "cold_function");

    // Fourth sample has perf's "[unknown]" placeholders, preserved as-is.
    assert_eq!(samples[3].stack.len(), 1);
    assert_eq!(samples[3].stack[0].symbol, "[unknown]");
    assert_eq!(samples[3].stack[0].dso, "[unknown]");
}

#[test]
fn convert_aggregates_identical_stacks() {
    let gz = convert(FIXTURE, &ConvertOptions::default()).unwrap();
    let mut decoder = GzDecoder::new(gz.as_slice());
    let mut raw = Vec::new();
    decoder.read_to_end(&mut raw).unwrap();
    let profile = firefox_to_pprof::proto::Profile::decode(raw.as_slice()).unwrap();

    // Two distinct hot/cold stacks + one unknown stack = 3 unique stacks.
    assert_eq!(profile.sample.len(), 3);
    // Sample values: [samples_count, cpu_nanoseconds_sum].
    assert_eq!(profile.sample_type.len(), 2);
    let total_count: i64 = profile.sample.iter().map(|s| s.value[0]).sum();
    assert_eq!(total_count, 4);
    let total_period: i64 = profile.sample.iter().map(|s| s.value[1]).sum();
    assert_eq!(total_period, 4_000_000);

    // Find the sample that aggregated two events (the hot stack).
    let hot = profile.sample.iter().find(|s| s.value[0] == 2).unwrap();
    assert_eq!(hot.value[1], 2_000_000);

    // Functions table includes both named symbols + the unknown one.
    let names: Vec<&str> = profile
        .function
        .iter()
        .map(|f| profile.string_table[f.name as usize].as_str())
        .collect();
    assert!(names.contains(&"hot_function"));
    assert!(names.contains(&"cold_function"));
    assert!(names.contains(&"[unknown]"));
}

#[test]
fn stats_detects_missing_period() {
    let samples = parse(NO_PERIOD).unwrap();
    let stats = Stats::from_samples(&samples);
    assert_eq!(stats.sample_count, 2);
    assert_eq!(stats.period_sum, 2); // every sample defaulted to 1
    assert!(stats.period_likely_missing());
    assert!(stats.unknown_ratio() < 0.01);
}

#[test]
fn stats_detects_high_unknown_ratio() {
    let samples = parse(MANY_UNKNOWNS).unwrap();
    let stats = Stats::from_samples(&samples);
    assert_eq!(stats.sample_count, 2);
    assert_eq!(stats.frame_count, 5);
    assert_eq!(stats.unknown_frame_count, 4);
    assert!(stats.unknown_ratio() > 0.5);
    assert!(!stats.period_likely_missing()); // periods are real here
}

#[test]
fn stats_clean_fixture_has_no_warnings() {
    let samples = parse(FIXTURE).unwrap();
    let stats = Stats::from_samples(&samples);
    assert!(!stats.period_likely_missing());
    // FIXTURE has one [unknown] frame out of 9 → well under 50 %.
    assert!(stats.unknown_ratio() < 0.5);
}
