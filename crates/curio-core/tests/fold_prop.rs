//! Property tests for fold semantics: any interleaving of set-membership
//! ops folds to the last op per (article, set), and `event_id` dedupe makes
//! replays idempotent.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use curio_core::events::FoldedState;
use curio_types::{CurioId, EventEnvelope, EventId, EventPayload, EventsSchemaV1Marker};
use proptest::prelude::*;

/// The membership-flipping op set, minus feeds (string-keyed, same shape).
#[derive(Debug, Clone, Copy)]
enum Op {
    Star,
    Unstar,
    ReadLaterAdd,
    ReadLaterRemove,
    Archive,
    Unarchive,
}

const OPS: [Op; 6] = [
    Op::Star,
    Op::Unstar,
    Op::ReadLaterAdd,
    Op::ReadLaterRemove,
    Op::Archive,
    Op::Unarchive,
];

fn payload(op: Op, curio_id: CurioId) -> EventPayload {
    match op {
        Op::Star => EventPayload::ArticleStarred {
            curio_id,
            tags: vec![],
        },
        Op::Unstar => EventPayload::ArticleUnstarred { curio_id },
        Op::ReadLaterAdd => EventPayload::ReadLaterAdded {
            curio_id,
            tags: vec![],
        },
        Op::ReadLaterRemove => EventPayload::ReadLaterRemoved { curio_id },
        Op::Archive => EventPayload::ArticleArchived { curio_id },
        Op::Unarchive => EventPayload::ArticleUnarchived { curio_id },
    }
}

fn envelope(op: Op, curio_id: CurioId) -> EventEnvelope {
    EventEnvelope {
        schema: EventsSchemaV1Marker,
        event_id: EventId::new(),
        ts: "2026-07-03T10:00:00.000Z".parse().unwrap(),
        event: payload(op, curio_id),
    }
}

/// The trivially-correct reference model: three bools per article,
/// mutated in order.
#[derive(Default, Clone, Copy)]
struct Model {
    starred: bool,
    read_later: bool,
    archived: bool,
}

impl Model {
    fn apply(&mut self, op: Op) {
        match op {
            Op::Star => self.starred = true,
            Op::Unstar => self.starred = false,
            Op::ReadLaterAdd => self.read_later = true,
            Op::ReadLaterRemove => self.read_later = false,
            Op::Archive => self.archived = true,
            Op::Unarchive => self.archived = false,
        }
    }
}

proptest! {
    /// Any interleaving of star/unstar (and friends) across several
    /// articles folds to exactly the last op per (article, set).
    #[test]
    fn fold_matches_the_last_op_model(
        ops in proptest::collection::vec((0usize..4, 0usize..OPS.len()), 0..64),
    ) {
        let ids: Vec<CurioId> = (0..4).map(|_| CurioId::new()).collect();
        let mut models = [Model::default(); 4];
        let mut events = Vec::with_capacity(ops.len());
        for (article, op_idx) in ops {
            let op = OPS[op_idx];
            models[article].apply(op);
            events.push(envelope(op, ids[article]));
        }

        let state = FoldedState::fold(events);
        for (id, model) in ids.iter().zip(models.iter()) {
            prop_assert_eq!(state.starred.contains(id), model.starred);
            prop_assert_eq!(state.read_later.contains(id), model.read_later);
            prop_assert_eq!(state.archived.contains(id), model.archived);
        }
    }

    /// Replaying any prefix of the stream after the full stream (an
    /// overlapping-cursor consumer) changes nothing: dedupe by event_id.
    #[test]
    fn replayed_prefixes_are_idempotent(
        ops in proptest::collection::vec((0usize..4, 0usize..OPS.len()), 1..48),
        prefix_frac in 0.0f64..=1.0,
    ) {
        let ids: Vec<CurioId> = (0..4).map(|_| CurioId::new()).collect();
        let events: Vec<EventEnvelope> = ops
            .iter()
            .map(|&(article, op_idx)| envelope(OPS[op_idx], ids[article]))
            .collect();

        let clean = FoldedState::fold(events.clone());

        #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let prefix_len = ((events.len() as f64) * prefix_frac) as usize;
        let mut replayed_stream = events.clone();
        replayed_stream.extend(events[..prefix_len.min(events.len())].iter().cloned());
        let replayed = FoldedState::fold(replayed_stream);

        prop_assert_eq!(&replayed.starred, &clean.starred);
        prop_assert_eq!(&replayed.read_later, &clean.read_later);
        prop_assert_eq!(&replayed.archived, &clean.archived);
        prop_assert_eq!(replayed.events_seen(), clean.events_seen());
    }
}
