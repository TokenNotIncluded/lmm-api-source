use super::*;
fn grant(limit: i64, used: i64, reset: Option<(i64, bool)>) -> FrozenGrant {
    FrozenGrant {
        id: 1,
        user_id: 2,
        plan_id: 3,
        window_start: 1,
        window_end: 2,
        limit_credits: limit,
        unlimited: false,
        used,
        reserved: 0,
        previous_reset_id: None,
        reset_basis: if reset.is_some() { "explicit" } else { "same" }.into(),
        reset_limit: reset.map_or(0, |v| v.0),
        reset_unlimited: reset.is_some_and(|v| v.1),
        grant: json!({}),
    }
}
#[test]
fn reset_basis_preserves_refund_rebase_and_unlimited_semantics() {
    assert_eq!(grant(400, 300, Some((200, false))).restored(), Ok(100));
    assert_eq!(grant(400, 20, Some((200, false))).restored(), Ok(0));
    assert_eq!(grant(400, 500, Some((200, false))).restored(), Ok(200));
    assert_eq!(grant(400, 300, None).restored(), Ok(300));
    assert_eq!(grant(400, 300, Some((0, true))).after_basis(), (0, true));
    assert_eq!(grant(400, 300, Some((0, true))).restored(), Ok(0));
    assert_eq!(checked_sum([i64::MAX, 1].into_iter()), Err(Error::Invalid));
}
#[test]
fn preview_binds_mode_expiry_consumption_and_generation() {
    let g = grant(400, 300, Some((200, false)));
    let initial = preview_digest("soft", 100, std::slice::from_ref(&g)).unwrap();
    assert_ne!(
        initial,
        preview_digest("soft", 101, std::slice::from_ref(&g)).unwrap()
    );
    assert_ne!(
        initial,
        preview_digest("hard", 0, std::slice::from_ref(&g)).unwrap()
    );
    let mut changed = g.clone();
    changed.previous_reset_id = Some(9);
    assert_ne!(initial, preview_digest("soft", 100, &[changed]).unwrap());
    let mut changed = g;
    changed.used = 299;
    assert_ne!(initial, preview_digest("soft", 100, &[changed]).unwrap());
}
