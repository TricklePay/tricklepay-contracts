# ADR: Keep the stream id in event data

**Status:** Accepted

## Context

The `Created` event indexes `sender` and `recipient`; `Withdrawn` indexes
`recipient`; and `Cancelled` indexes `sender`. Each event includes the stream
`id` in its data. An indexer following one stream therefore cannot filter RPC
results by id and must fetch the relevant events and filter their payloads.

## Options considered

1. **Add `id` as a topic to the existing events.** This would let an indexer
   filter directly by stream id. It would also change the public event schema:
   the id would move from event data into the indexed topics, and each event
   would have an additional topic.
2. **Keep the current schema.** Consumers continue filtering event data for
   one stream. They can also discover ids from `Created` events and track
   subsequent events in their own index.

## Decision

Keep `id` in event data and do not add it as a topic to the existing events.
Direct per-stream RPC filtering would be useful, but does not justify changing
the event schema without coordinating a migration across consumers.

## Consumers and migration cost

The affected consumers include the `tricklepay-backend` event decoder and any
other indexer, generated client, explorer, or integration that parses these
events. The backend decoder reads topic positions; adding `id` as a topic
requires updating its topic parsing and event-data decoding for `Created`,
`Withdrawn`, and `Cancelled`. Other consumers that rely on the current topic
count or data layout would need corresponding updates. Deployments and
consumers would need to agree on when the new schema is active; otherwise,
decoders expecting the current layout could fail to parse events.

With this decision, no ABI change is made and no decoder migration is
required. If the decision is revisited, the event-schema change must be
identified as an ABI change in the changelog, and the backend decoder update
must be included in the migration plan.
