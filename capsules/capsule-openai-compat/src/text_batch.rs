//! Coalesce adjacent text within a received HTTP chunk, not across reads.
//!
//! The provider owns these semantics. IPC and the kernel remain payload-blind.
//! Flush before every non-text event and at every read boundary: slow streams
//! acquire no extra buffering delay, and tools/usage/terminal cannot overtake text.

use astrid_sdk::types::StreamEvent;

/// Bounds retained text and the size of an emitted combined delta.
const MAX_TEXT_BYTES: usize = 8 * 1024;

#[derive(Default)]
pub(super) struct TextBatch {
    text: String,
}

impl TextBatch {
    pub(super) fn push<E>(
        &mut self,
        event: StreamEvent,
        emit: &mut impl FnMut(StreamEvent) -> Result<(), E>,
    ) -> Result<(), E> {
        let StreamEvent::TextDelta(text) = event else {
            self.flush(emit)?;
            return emit(event);
        };
        let mut remaining = text.as_str();
        while !remaining.is_empty() {
            let available = MAX_TEXT_BYTES - self.text.len();
            let mut end = remaining.len().min(available);
            while !remaining.is_char_boundary(end) {
                end -= 1;
            }
            if end == 0 {
                self.flush(emit)?;
                continue;
            }
            self.text.push_str(&remaining[..end]);
            remaining = &remaining[end..];
            if self.text.len() == MAX_TEXT_BYTES {
                self.flush(emit)?;
            }
        }
        Ok(())
    }

    pub(super) fn flush<E>(
        &mut self,
        emit: &mut impl FnMut(StreamEvent) -> Result<(), E>,
    ) -> Result<(), E> {
        if self.text.is_empty() {
            return Ok(());
        }
        emit(StreamEvent::TextDelta(std::mem::take(&mut self.text)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fast_burst_is_complete_bounded_and_finishes_after_text() {
        let mut batch = TextBatch::default();
        let mut events = Vec::new();
        let mut emit = |event| {
            events.push(event);
            Ok::<_, ()>(())
        };
        let mut expected = String::new();
        for index in 0..4096 {
            let delta = format!("unit-{index:05} ");
            expected.push_str(&delta);
            batch
                .push(StreamEvent::TextDelta(delta), &mut emit)
                .unwrap();
            assert!(batch.text.len() <= MAX_TEXT_BYTES);
        }
        batch.push(StreamEvent::Done, &mut emit).unwrap();
        assert!(matches!(events.last(), Some(StreamEvent::Done)));
        assert!(
            events.len() < 10,
            "one downstream call per token must be avoided"
        );
        let actual: String = events
            .iter()
            .filter_map(|event| match event {
                StreamEvent::TextDelta(text) => {
                    assert!(text.len() <= MAX_TEXT_BYTES);
                    Some(text.as_str())
                }
                _ => None,
            })
            .collect();
        assert_eq!(actual, expected);
    }

    #[test]
    fn unicode_and_tool_boundaries_preserve_order() {
        let mut batch = TextBatch::default();
        let mut events = Vec::new();
        let mut emit = |event| {
            events.push(event);
            Ok::<_, ()>(())
        };
        let text = "🦀".repeat(MAX_TEXT_BYTES);
        batch
            .push(StreamEvent::TextDelta(text.clone()), &mut emit)
            .unwrap();
        batch
            .push(
                StreamEvent::ToolCallStart {
                    id: "call".into(),
                    name: "tool".into(),
                },
                &mut emit,
            )
            .unwrap();
        batch
            .push(StreamEvent::TextDelta("after".into()), &mut emit)
            .unwrap();
        batch.flush(&mut emit).unwrap();
        assert!(
            matches!(&events[events.len()-2], StreamEvent::ToolCallStart { id, .. } if id == "call")
        );
        assert!(matches!(events.last(), Some(StreamEvent::TextDelta(text)) if text == "after"));
        let before: String = events[..events.len() - 2]
            .iter()
            .map(|event| match event {
                StreamEvent::TextDelta(text) => text.as_str(),
                _ => panic!("text before tool"),
            })
            .collect();
        assert_eq!(before, text);
    }

    #[test]
    fn read_boundary_flushes_immediately_and_failure_stops_later_events() {
        let mut batch = TextBatch::default();
        let mut events = Vec::new();
        let mut emit = |event| {
            events.push(event);
            Ok::<_, ()>(())
        };
        batch
            .push(StreamEvent::TextDelta("slow token".into()), &mut emit)
            .unwrap();
        batch.flush(&mut emit).unwrap();
        batch
            .push(StreamEvent::TextDelta("unsent".into()), &mut emit)
            .unwrap();
        assert_eq!(events.len(), 1);
        let mut calls = 0;
        let error = batch.push(StreamEvent::Done, &mut |_| {
            calls += 1;
            Err("unavailable")
        });
        assert_eq!(error, Err("unavailable"));
        assert_eq!(calls, 1, "terminal cannot pass a failed text publication");
    }
}
