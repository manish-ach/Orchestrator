# Product

## Register

product

## Users

A four-person student team (coordinator, parser, executor, dashboard owners) and their project supervisor / defense panel. Two contexts: day-to-day, a developer glances at the dashboard while working in a terminal next to it to check whether a pipeline run passed and why it failed; at the defense, it is projected in a bright room while a live run executes across five physical worker laptops.

## Product Purpose

The dashboard is the read-only window onto a distributed CI/CD orchestrator (Rust coordinator, Python pipeline parser, FastAPI command executor). It shows pipeline runs, their stages and jobs, worker health, and job logs — fed exclusively by the coordinator's REST API, refreshed by polling. Success: a viewer can answer "is it green, what broke, which worker ran it" in under five seconds, and the live demo makes the distributed system's behavior visible (a run progressing, a worker dying and being reaped).

## Brand Personality

Instrument panel. Calibrated, dense, legible — a workshop tool, not a SaaS product. Creativity lives in how well the data is visualized (stage strips, duration timelines, live log tail), never in decoration. Three words: precise, quiet, trustworthy.

## Anti-references

- Generic AI-generated SaaS dashboards: hero metric cards with gradient accents, icon+heading card grids, glassmorphism, purple-on-white.
- The slide-12 concept mockups' scope creep (health scores, search, settings) — the real thing is three pages that show real data.
- Anything framework-shaped: this is pure HTML/CSS/JS with no build step; the design must look deliberate at that fidelity.

## Design Principles

1. **Data is the interface.** Every pixel either shows pipeline/worker state or gets out of the way. Tables and timelines over cards.
2. **Status is a vocabulary, not a color.** Passed/failed/running/pending each get a fixed glyph + color + word, used identically everywhere; never color alone.
3. **Honest about its medium.** System fonts, hairline rules, monospace data — a static page served by the coordinator, and proud of it.
4. **Teach through empty states.** When there are no runs, show the curl command that creates one.
5. **Demo-first.** Anything that moves (polling readout, running-job pulse, log tail) must read from across a defense room.

## Accessibility & Inclusion

WCAG AA. Status never conveyed by color alone (paired glyph + word). Body text ≥4.5:1; muted text ≥4.5:1 on white. Reduced-motion alternative for every animation. Fully keyboard-navigable tables and links.
