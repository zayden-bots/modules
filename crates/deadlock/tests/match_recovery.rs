use deadlock::deliver::allowed_matches;
use deadlock::match_poll::{SWEEP_INTERVAL, sweep_due};
use jiff::{SignedDuration, Timestamp};

const fn at(seconds: i64) -> Timestamp {
    Timestamp::constant(seconds, 0)
}

#[test]
fn a_sweep_is_due_when_none_ran_or_the_interval_has_elapsed() {
    let now = at(1_800_000_000);

    assert!(sweep_due(None, now));
    assert!(!sweep_due(
        Some(now.checked_sub(SignedDuration::from_mins(59)).unwrap()),
        now
    ));
    assert!(sweep_due(Some(now.checked_sub(SWEEP_INTERVAL).unwrap()), now));
}

#[test]
fn only_the_newest_recovered_matches_are_allowed() {
    let entries: Vec<(u64, Timestamp, bool)> =
        (1..=5).map(|n| (n, at(1_000 + i64::try_from(n).unwrap()), true)).collect();

    let allowed = allowed_matches(&entries, 3);

    assert_eq!(allowed.len(), 3);
    assert!(allowed.contains(&5) && allowed.contains(&4) && allowed.contains(&3));
}

#[test]
fn non_recovered_matches_are_always_allowed_on_top_of_the_cap() {
    let mut entries: Vec<(u64, Timestamp, bool)> =
        (1..=5).map(|n| (n, at(1_000 + i64::try_from(n).unwrap()), true)).collect();
    entries.push((90, at(10), false));
    entries.push((91, at(20), false));

    let allowed = allowed_matches(&entries, 3);

    assert_eq!(allowed.len(), 5);
    assert!(allowed.contains(&90) && allowed.contains(&91));
    assert!(!allowed.contains(&1) && !allowed.contains(&2));
}

#[test]
fn a_duplicated_match_id_counts_once() {
    let entries = vec![
        (1, at(100), true),
        (1, at(100), true),
        (2, at(90), true),
        (3, at(80), true),
    ];

    let allowed = allowed_matches(&entries, 3);

    assert_eq!(allowed.len(), 3);
}
