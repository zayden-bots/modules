use deadlock::match_poll::{next_streak, streak_from};

#[test]
fn a_streak_is_rebuilt_from_results_in_start_order() {
    assert_eq!(streak_from(&[]), 0);
    assert_eq!(streak_from(&[true, true, false, false, false]), -3);
    assert_eq!(streak_from(&[false, true, true]), 2);
}

#[test]
fn a_win_extends_a_winning_streak_or_starts_one() {
    assert_eq!(next_streak(0, true), 1);
    assert_eq!(next_streak(3, true), 4);
    assert_eq!(next_streak(-2, true), 1);
}

#[test]
fn a_loss_extends_a_losing_streak_or_starts_one() {
    assert_eq!(next_streak(0, false), -1);
    assert_eq!(next_streak(3, false), -1);
    assert_eq!(next_streak(-2, false), -3);
}

#[test]
fn streaks_saturate_instead_of_overflowing() {
    assert_eq!(next_streak(i32::MAX, true), i32::MAX);
    assert_eq!(next_streak(i32::MIN, false), i32::MIN);
}
