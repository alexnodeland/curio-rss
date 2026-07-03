# Governance

Curio is a [BDFL](https://en.wikipedia.org/wiki/Benevolent_dictator_for_life)
project. **Alex Nodeland** is the maintainer and has final say on scope,
design, and releases.

## How decisions are made

- Direction-setting design lives in `docs/design/` (architecture, roadmap,
  contracts, decisions). Material changes to those documents are maintainer
  decisions, made in the open via PRs.
- The published contracts (`curio.frontmatter.v1`, `curio.events.v1`) carry
  a stricter bar: they are versioned-immutable once consumers exist. See
  `schemas/README.md` and `.claude/skills/contract-change/`.
- Everything else follows normal PR review; [CONTRIBUTING.md](CONTRIBUTING.md)
  defines the mechanical floor (CI, lints, hermetic tests).

## Scope discipline

The roadmap ([docs/design/roadmap.md](docs/design/roadmap.md)) names what v1
is and — just as deliberately — what it is not (no sync, no server head, no
enrichment providers). PRs that expand scope ahead of the roadmap will be
declined regardless of quality; open a design discussion first.

## Succession

If the maintainer becomes unavailable for an extended period, the MIT OR
Apache-2.0 licensing guarantees the project is forkable by anyone at any time
— that is the succession plan.
