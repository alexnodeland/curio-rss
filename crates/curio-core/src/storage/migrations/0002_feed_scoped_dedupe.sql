-- Migration 0002 — feed-scoped article dedupe keys.
--
-- The v1 schema made dedupe_key globally UNIQUE and matched upserts on it
-- alone, so colliding guids from two different feeds (sequential-integer
-- guids are common; guids are attacker-choosable) silently overwrote each
-- other's content on every refresh. The stored key is now namespaced by
-- provenance at write time — 'f<feed_id>:' for feed articles, 'm:' for
-- manual saves — which scopes the match per feed while keeping the
-- existing UNIQUE constraint as defense in depth.
--
-- Existing rows are rewritten with the same transform the repo layer now
-- applies on insert. Old keys were globally unique, and the transform is
-- injective per provenance class, so no collisions can arise here.

UPDATE articles
   SET dedupe_key = 'f' || feed_id || ':' || dedupe_key
 WHERE feed_id IS NOT NULL;

UPDATE articles
   SET dedupe_key = 'm:' || dedupe_key
 WHERE feed_id IS NULL;
