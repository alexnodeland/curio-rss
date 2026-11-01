---
schema: curio.frontmatter.v1
curio_id: 0197b2c4-8f3e-7cc1-a5d2-3e9f10aa4b6d
title: &title Old title
source: https://example.com/article
feed: &feed https://example.com/feed.xml
published: null
saved: 2026-07-03T09:15:00.123Z
tags: &tags
  - rust
checksum: sha256:0000000000000000000000000000000000000000000000000000000000000000
base: &base
  color: blue
  size: 3
derived:
  <<: *base
  color: red
same_tags: *tags
title_copy: *title
feed_copy: *feed
list_of_refs: [*base, *title]
---

<!-- curio:managed:begin v1 -->
old body
<!-- curio:managed:end -->
