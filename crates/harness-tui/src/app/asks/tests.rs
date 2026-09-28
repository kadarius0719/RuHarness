//! The chat's side of the cockpit (docs/CHAT-PANE-DESIGN.md §9 "app").

use super::*;

#[test]
fn a_burst_is_keys_within_five_ms() {
    let t0 = Instant::now();
    let mut a = Asks::default();
    a.key_read(t0, false);
    assert!(!a.burst);
    a.key_read(t0 + Duration::from_millis(3), false);
    assert!(a.burst, "the last key of a burst is in it");
    a.key_read(t0 + Duration::from_millis(20), false);
    assert!(!a.burst);
    a.key_read(t0 + Duration::from_millis(40), true);
    assert!(a.burst, "input pending after the read");
}
