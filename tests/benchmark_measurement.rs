#[allow(dead_code)]
#[path = "../benchmarks/wsldb/measurement.rs"]
mod measurement;

use measurement::MeasurementPlan;

#[test]
fn defaults_preserve_frozen_warm_profile() {
    let plan = MeasurementPlan::from_values(None, None, None).unwrap();
    assert!(plan.warmup());
    assert_eq!(plan.runs().collect::<Vec<_>>(), vec![1, 2, 3, 4, 5]);
}

#[test]
fn one_cold_process_keeps_its_global_run_id() {
    let plan = MeasurementPlan::from_values(Some("0"), Some("1"), Some("0")).unwrap();
    assert!(!plan.warmup());
    assert_eq!(plan.first_run(), 0);
    assert_eq!(plan.runs().collect::<Vec<_>>(), vec![0]);
}

#[test]
fn invalid_or_overflowing_plans_fail_closed() {
    assert!(MeasurementPlan::from_values(Some("-1"), None, None).is_err());
    assert!(MeasurementPlan::from_values(None, Some("0"), None).is_err());
    assert!(MeasurementPlan::from_values(None, None, Some("yes")).is_err());
    assert!(MeasurementPlan::from_values(Some("4294967295"), Some("2"), None).is_err());
}
