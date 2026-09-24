---
version: 1
slug: "web-index-html"
primary_target: "web/index.html"
related_targets: ["web/src/style.css","web/src/main.ts"]
---

# Surface brief: web (marketing site)

Scope: `web/index.html`, `web/src/style.css`, `web/src/main.ts` — the whole static site.
Visitor mode: Persuade.

Audience: developers who live in the terminal and want recurring work to run as
defined, repeatable workflows. Job: understand what Pulse is from its own
artifact, believe the run and report are real, copy the install command.
Constraints: PRODUCT.md truth only; ECG/heartbeat motif is a committed brand
vocabulary; `web` stays self-contained; oxlint/oxfmt; Vite `base: './'`.

## Direction contract

THESIS: The workflow file is the whole interface, so the page is the file:
ship.yml at monument scale, its `#` comments carrying the pitch, the install
command as the command line beside it. Refuses the hero-plus-feature-cards
arrangement the category ships.

OWN-WORLD: dark editor ground (#101512), light ink, one committed bright
emerald (#34d399) for goals, ticks, and cost; comment gray for annotation;
Martian Mono for every artifact (files, commands, listings), Spline Sans for
human prose; file chrome (editor tabs, status line carrying the ECG beat) is
the only furniture; no header, no cards; one vertical scroll, animated.
Owner-pinned after the round: dark tone, header removed, brand in footer,
playground section removed, scroll animation driven from JS so it shows in
 every browser.

STORY: the visitor reads a real workflow file and understands the product from
inside its interface; watches that file run, goals ticking and tools streaming
in as comment lines; believes the report because they read an excerpt and the
ls listing of past runs; acts by copying the mise install command, reachable
in the first viewport and again at the close.

FORM: The Annotated Workflow File (dotfiles master), candidate 1 of 7,
user-locked over the assigned Run Board; seed key e90a0116, code-led build.

FIRST VIEWPORT: no header; the page opens on the mise prompt line with Copy,
then ship.yml filling the screen, display-scale comment lines as the headline,
name, model, steps with trailing comments; the file executes on load, goal
lines ticking reached while the ECG status line beats at the file's foot.

FINISH: unreviewed and undocumented is unfinished; this build ends with the
finish review, the verdict, DESIGN.md, and every shipping raster carrying its
provenance.

Unresolved: none material; fonts are Google-hosted (Martian Mono, Spline Sans)
to be self-hosted as woff2 in `web/src/fonts/`.
