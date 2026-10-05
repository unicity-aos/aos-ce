//! Bounded collection for a fan-out with no authoritative responder count.

use std::time::Duration;

use astrid_sdk::ipc::PollResult;

/// A quiet receive is not completion: providers may still be queued for CPU.
/// Use actual monotonic elapsed time, not the requested receive timeout, since
/// a receive containing messages can return immediately.
pub(super) fn collect<E>(
    window: Duration,
    slice: Duration,
    mut receive: impl FnMut(u64) -> Result<PollResult, E>,
    mut consume: impl FnMut(PollResult),
    mut now: impl FnMut() -> Duration,
) -> Result<(), E> {
    let started = now();
    loop {
        let remaining = window.saturating_sub(now().saturating_sub(started));
        if remaining.is_zero() {
            return Ok(());
        }
        let timeout = remaining.min(slice).as_millis();
        // A sub-millisecond remainder cannot be expressed by recv's ABI.
        if timeout == 0 {
            return Ok(());
        }
        consume(receive(u64::try_from(timeout).unwrap_or(u64::MAX))?);
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use astrid_sdk::ipc::{Message, PrincipalAttribution};

    use super::*;

    fn batch(nonempty: bool) -> PollResult {
        PollResult {
            messages: if nonempty {
                vec![Message {
                    topic: "tool.v1.response.describe.provider".into(),
                    payload: "{}".into(),
                    source_id: "provider".into(),
                    principal: PrincipalAttribution::Verified("test".into()),
                }]
            } else {
                Vec::new()
            },
            dropped: 0,
            lagged: 0,
        }
    }

    #[test]
    fn quiet_slice_does_not_discard_a_later_provider() {
        let elapsed = Cell::new(0_u64);
        let calls = Cell::new(0);
        let mut responders = 0;
        collect(
            Duration::from_millis(300),
            Duration::from_millis(100),
            |timeout| {
                calls.set(calls.get() + 1);
                elapsed.set(elapsed.get() + timeout);
                Ok::<_, ()>(batch(matches!(calls.get(), 1 | 3)))
            },
            |result| responders += result.messages.len(),
            || Duration::from_millis(elapsed.get()),
        )
        .unwrap();
        assert_eq!(responders, 2);
        assert_eq!(calls.get(), 3);
        assert_eq!(elapsed.get(), 300);
    }

    #[test]
    fn immediate_reply_does_not_spend_the_requested_timeout() {
        let elapsed = Cell::new(0_u64);
        let mut responders = 0;
        collect(
            Duration::from_millis(300),
            Duration::from_millis(100),
            |_| {
                elapsed.set(elapsed.get() + 10);
                Ok::<_, ()>(batch(true))
            },
            |result| responders += result.messages.len(),
            || Duration::from_millis(elapsed.get()),
        )
        .unwrap();
        assert_eq!(responders, 30);
    }

    #[test]
    fn receive_failure_is_returned_not_reported_as_complete() {
        let result = collect(
            Duration::from_millis(300),
            Duration::from_millis(100),
            |_| Err::<PollResult, _>("subscription closed"),
            |_| panic!("failed receive must not be consumed"),
            || Duration::ZERO,
        );
        assert_eq!(result, Err("subscription closed"));
    }
}
