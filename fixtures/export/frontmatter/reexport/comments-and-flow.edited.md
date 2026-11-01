---
# A comment above everything.
schema: curio.frontmatter.v1   # trailing comment
curio_id: 0197b2c4-8f3e-7cc1-a5d2-3e9f10aa4b6d
title: Old title
source: https://example.com/article
feed: https://example.com/feed.xml
published: 2026-07-01T12:00:00.000Z
saved: 2026-07-03T09:15:00.123Z
tags: [rust, "old tag"]   # flow sequence
checksum: sha256:0000000000000000000000000000000000000000000000000000000000000000
# User keys below.
rating: 5
status: {stage: reading, done: false}   # flow mapping
aliases: [ "First", 'Second', third ]
nested_flow: {list: [1, 2, {deep: x}], empty: [], none: {}}
reviewed: # value on the next line
  2026-07-04
---

User preamble above the region.

<!-- curio:managed:begin v1 -->
old body
<!-- curio:managed:end -->

## My notes

Companion text survives.
