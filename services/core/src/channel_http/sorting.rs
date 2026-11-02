//! Unknown values stay last in either direction; equal values use ascending ID.
use super::Channel;
use std::cmp::Ordering;

pub(super) fn optional<T>(
    a: Option<T>,
    b: Option<T>,
    descending: bool,
    compare: impl FnOnce(T, T) -> Ordering,
) -> Ordering {
    match (a, b) {
        (Some(a), Some(b)) => {
            let order = compare(a, b);
            if descending { order.reverse() } else { order }
        }
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

pub(super) fn compare(
    a: &Channel,
    b: &Channel,
    field: &str,
    unit: Option<&str>,
    descending: bool,
) -> Ordering {
    let stats_a = a.statistics.as_ref();
    let stats_b = b.statistics.as_ref();
    let result = match field {
        "lifetime" => optional(
            stats_a.and_then(|s| s.amount.as_ref()),
            stats_b.and_then(|s| s.amount.as_ref()),
            descending,
            Ord::cmp,
        ),
        "balance" => {
            let value = |c: &Channel| {
                c.statistics
                    .as_ref()
                    .and_then(|s| s.observation.as_ref())
                    .filter(|v| v.unit_label.as_deref() == unit && v.kind != "unknown")
                    .and_then(|v| v.number)
            };
            optional(value(a), value(b), descending, |a, b| {
                a.partial_cmp(&b).unwrap_or(Ordering::Equal)
            })
        }
        "response_time" => optional(
            stats_a.and_then(|s| s.response_time),
            stats_b.and_then(|s| s.response_time),
            descending,
            |a, b| a.cmp(&b),
        ),
        "test_time" => optional(
            stats_a.and_then(|s| s.test_time),
            stats_b.and_then(|s| s.test_time),
            descending,
            |a, b| a.cmp(&b),
        ),
        "priority" => optional(a.priority, b.priority, descending, |a, b| a.cmp(&b)),
        "name" => {
            if descending {
                b.name.cmp(&a.name)
            } else {
                a.name.cmp(&b.name)
            }
        }
        _ => {
            if descending {
                b.id.cmp(&a.id)
            } else {
                a.id.cmp(&b.id)
            }
        }
    };
    result.then(a.id.cmp(&b.id))
}

#[cfg(test)]
mod tests;
