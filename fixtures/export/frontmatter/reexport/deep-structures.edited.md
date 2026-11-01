---
schema: curio.frontmatter.v1
curio_id: 0197b2c4-8f3e-7cc1-a5d2-3e9f10aa4b6d
title: Old title
source: https://example.com/article
feed: null
published: null
saved: 2026-07-03T09:15:00.123Z
tags: []
checksum: sha256:0000000000000000000000000000000000000000000000000000000000000000
projects:
- name: curio
  links:
    - https://example.com/one
    - {href: "https://example.com/two", rel: alternate}
  meta:
    stars: 12
    score: 4.50
    ratio: 1.0e2
- - nested
  - - deeper
-
1: numeric key
1.5: float key
true: bool key
null: null key
"": empty key
zz_unicode: "日本語 🦀"
---

<!-- curio:managed:begin v1 -->
old body
<!-- curio:managed:end -->
