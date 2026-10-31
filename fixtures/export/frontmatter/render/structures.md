---
schema: curio.frontmatter.v1
curio_id: 0197b2c4-8f3e-7cc1-a5d2-3e9f10aa4b6d
title: Article title
source: https://example.com/article
feed: https://example.com/feed.xml
published: 2026-07-01T12:00:00.000Z
saved: 2026-07-03T09:15:00.123Z
tags:
- rust
- databases
checksum: sha256:9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08
'': empty key
' space key': v
'#hash key': v
'- dash key': v
'123': numeric key
? LLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLL
: key over 128 bytes
? MMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMM
: value: map under a long key
? NNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNN
: - seq
  - under
  - long key
'a: b': colon key
kkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkk: key of exactly 128 bytes
? |-
  multi
  line key
: value
'true': bool key
x_empty_map: {}
x_empty_seq: []
x_false: false
x_float_big: 1e300
x_float_frac: 123.456
x_float_neg_zero: -0.0
x_float_one: 1.0
x_float_small: 1e-7
x_float_tenth: 0.1
x_i64_min: -9223372036854775808
x_int: 0
x_map:
  a: 1
  b: 2
  nested:
    deeper:
      deepest: yes
x_map_of_seq:
  empty: []
  list:
  - x
  - y
x_multiline_deep:
- text: |-
    a
    b
- - - |-
      c
      d
x_multiline_in_map:
  leading: |2-
     a
    b
  text: |
    one
    two
x_multiline_in_seq:
- |-
  one
  two
- plain
x_neg: -1
x_null: null
x_seq:
- a
- 1
- true
- null
- 2.5
x_seq_of_map:
- k: v
  n: 1
- {}
- only: one
x_seq_of_seq:
- - a
  - b
- []
- - - deep
x_single_key_map:
  '!tag': looks like a tag
x_true: true
x_u64_max: 18446744073709551615
'~': tilde key
? 日日日日日日日日日日日日日日日日日日日日日日日日日日日日日日日日日日日日日日日日日日日
: 43 three-byte chars = 129 bytes
---

<!-- curio:managed:begin v1 -->
Body line one.

Body line two.
<!-- curio:managed:end -->
