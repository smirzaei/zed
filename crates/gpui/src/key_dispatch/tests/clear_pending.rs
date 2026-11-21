use super::*;

#[crate::test]
fn test_clearing_pending_input_drops_it_without_replay(cx: &mut TestAppContext) {
    let (cx, action_count, secondary_action_count) = setup_pending_input_timeout_test(cx);
    struct PendingInputObserver {
        _subscription: Subscription,
    }
    let pending_input_changes = Rc::new(Cell::new(0));
    let _observer = cx.update(|window, cx| {
        cx.new(|cx| PendingInputObserver {
            _subscription: cx.observe_pending_input(window, {
                let pending_input_changes = pending_input_changes.clone();
                move |_, _, _| pending_input_changes.set(pending_input_changes.get() + 1)
            }),
        })
    });
    simulate_pending_binding(cx);
    cx.run_until_parked();
    let changes_before_clear = pending_input_changes.get();

    cx.update(|window, cx| {
        assert!(window.has_pending_keystrokes());
        window.clear_pending_keystrokes(cx);
        assert!(window.pending_input_is_none());
        assert_eq!(pending_input_changes.get(), changes_before_clear);
    });
    cx.run_until_parked();
    assert!(pending_input_changes.get() > changes_before_clear);

    cx.simulate_keystrokes("x");
    cx.executor()
        .advance_clock(crate::PENDING_INPUT_TIMEOUT * 3 / 4);
    // A timeout left over from the clear would flush this sequence early.
    simulate_pending_binding(cx);
    cx.executor()
        .advance_clock(crate::PENDING_INPUT_TIMEOUT * 3 / 4);
    cx.run_until_parked();

    cx.update(|window, _| assert!(window.has_pending_keystrokes()));
    assert_eq!(action_count.get(), 0);
    assert_eq!(secondary_action_count.get(), 0);
}
