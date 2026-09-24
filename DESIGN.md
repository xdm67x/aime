---
name: Pulse
description: Headless workflow runner; the annotated workflow file is the whole interface.
colors:
  ground: "#f4f4f0"
  surface-raised: "#fbfbf9"
  surface-chrome: "#ecece6"
  ink: "#161b18"
  prose: "#4a534d"
  comment: "#666f69"
  accent-emerald: "#047857"
  accent-emerald-soft: "rgba(4, 120, 87, 0.08)"
  accent-emerald-line: "rgba(4, 120, 87, 0.45)"
  hairline: "#e0e2da"
  on-accent: "#ffffff"
typography:
  display:
    fontFamily: "'Martian Mono', ui-monospace, 'SF Mono', Menlo, monospace"
    fontSize: "clamp(26px, 4.6vw, 48px)"
    fontWeight: 500
    lineHeight: 1.25
    letterSpacing: "-0.01em"
  artifact:
    fontFamily: "'Martian Mono', ui-monospace, 'SF Mono', Menlo, monospace"
    fontSize: "clamp(14px, 1.7vw, 19px)"
    fontWeight: 400
    lineHeight: 1.75
  headline:
    fontFamily: "'Spline Sans', ui-sans-serif, system-ui, sans-serif"
    fontSize: "clamp(26px, 3.4vw, 40px)"
    fontWeight: 600
    lineHeight: 1.12
    letterSpacing: "-0.02em"
  body:
    fontFamily: "'Spline Sans', ui-sans-serif, system-ui, sans-serif"
    fontSize: "15px"
    fontWeight: 400
    lineHeight: 1.6
  label:
    fontFamily: "'Martian Mono', ui-monospace, 'SF Mono', Menlo, monospace"
    fontSize: "11.5px"
    fontWeight: 400
    lineHeight: 1.4
rounded:
  window: "10px"
  pill: "999px"
  code: "5px"
spacing:
  gutter: "clamp(20px, 5vw, 56px)"
  section: "clamp(80px, 11vh, 130px)"
components:
  button-copy:
    backgroundColor: "transparent"
    textColor: "{colors.accent-emerald}"
    rounded: "{rounded.pill}"
    padding: "8px 16px"
  button-copy-hover:
    backgroundColor: "{colors.accent-emerald}"
    textColor: "{colors.on-accent}"
  file-window:
    backgroundColor: "{colors.surface-raised}"
    textColor: "{colors.ink}"
    rounded: "{rounded.window}"
    padding: "clamp(22px, 4vw, 40px) clamp(20px, 4vw, 44px)"
---

# Design System: Pulse

## Overview

**Creative North Star: "The Annotated Workflow File"**

Pulse's interface is a YAML file, so the page is one too. Everything a
computer writes (workflow files, shell commands, run traces, reports,
directory listings) is set in Martian Mono inside file windows with editor
chrome; everything a human says (the pitch, section headings, annotations)
is Spline Sans on the open ground. The site is a single light vertical
scroll: editor paper, one emerald accent that means something (keys, goals
reached, costs, the heartbeat), and hairlines instead of shadows. No header,
no navigation apparatus, no logo in the footer; the one card grid is the
workflow trio (goal, model, tools), built in the file-window grammar. The
workflow file in the first viewport executes as you watch, and the heartbeat
(ECG line) marks every live run.

**Key Characteristics:**
- **Dark editor ground with three tonal steps (ground, chrome, raised); no shadows anywhere** —
  superseded by the owner's light steer: editor paper ground, chrome strips
  one step darker, raised file bodies near-white; still no shadows
- One accent: emerald, carrying meaning only (keys, success ticks, cost, the beat)
- Two type voices: Martian Mono for machine artifacts, Spline Sans for human prose
- File chrome (tab bar, body, status line) is the only container pattern
- The ECG pulse line is the brand mark and appears wherever a run is live

## Colors

A light instrument palette: editor paper for surfaces, near-black ink for
text, and one deep emerald reserved for what matters.

### Primary
- **Signal Emerald** (#34d399): The only saturated color. YAML keys, goal
  ticks, costs, the prompt `$`, the ECG line, and the install command's
  Copy button. Its rarity is the point.

### Neutral
- **Editor Paper** (#f4f4f0): Page background, warm-neutral light editor ground.
- **Chrome** (#ecece6): File window strips (tab bars, status lines), one tonal step darker than the paper.
- **Raised** (#fbfbf9): File body surface, near-white: what floats is lighter.
- **Ink** (#161b18): Primary text inside and outside artifacts.
- **Prose** (#4a534d): Human-sentence text on the ground.
- **Comment** (#666f69): Annotation gray: trailing `#` comments, metadata, hints.
- **Hairline** (#e0e2da): Every border and rule; 1px only.

### Named Rules
**The One Accent Rule.** Emerald marks meaning (a key, a goal reached, a
cost, a live run) and is never spent on decoration, fills, or gradients.
One owner-approved exception: emerald-family text gradients on the hero
display (the goal word's landing), always emerald to mint, never a second
hue.
**The Artifact Rule.** Content Pulse itself produces (files, commands,
reports, listings) renders inside file chrome; human commentary stays on
the open ground, never inside a window.

## Typography

**Display Font:** Martian Mono (with ui-monospace, SF Mono, Menlo fallback)
**Body Font:** Spline Sans (with ui-sans-serif, system-ui fallback)
**Label/Mono Font:** Martian Mono at small sizes for chrome and metadata

**Character:** A wide, square-cut mono carries the machine's voice at
display scale; a quiet humanist grotesque answers for people. The pairing
is the product's argument: you write a file, the machine runs it.

### Hierarchy
- **Display** (500, clamp(26px, 4.6vw, 48px), 1.25): The hero title, in mono.
- **Artifact** (400, clamp(14px, 1.7vw, 19px), 1.75): File body lines, commands, listings.
- **Headline** (600, clamp(26px, 3.4vw, 40px), 1.12, -0.02em): Section headings, Spline Sans.
- **Body** (400, 15px, 1.6): Human prose, max 56ch.
- **Label** (400, 11.5-12.5px): File chrome, status lines, hints, footer.

### Named Rules
**The Two Voices Rule.** Martian Mono is for what a computer writes;
Spline Sans is for what a person says. Never set a human sentence in mono,
never set file content in the sans.

## Layout

One vertical scroll, no header. A centered column (max 1080px, gutters
clamp(20px, 5vw, 56px)) over full-bleed sections separated by 1px
hairlines. Sections breathe clamp(80px, 11vh, 130px) vertical padding,
with more space above a heading than below it. File windows sit in the
column at max 860px; the `ls` listing centers at 760px. Below 900px the
report columns stack; below 720px the annotation rows and long YAML
comments stack. The install command is reachable in the first viewport
and again at the close.

## Elevation & Depth

No shadows exist in this system. Depth is tonal layering in three steps:
the paper ground (#f4f4f0) for the page, chrome strips (#ecece6) one step
darker for file furniture, and near-white raised bodies (#fbfbf9) for file
content, each separated by a 1px hairline (#e0e2da). What floats is lighter
than what it floats on, never blurred.

### Named Rules
**The No-Shadow Rule.** Never add box-shadow; a new surface gets a new
tonal step plus a hairline, or it does not exist.

## Shapes

Calm geometry: file windows and the report artifact round 10px; the Copy
button is a pill (999px) as the sole rounded control; inline code chips
round 5px. Tabs are square-edged with a right hairline and an inset 2px
emerald underline on the active tab. The ECG line is the only organic
form on the page.

## Components

### Buttons
- **Shape:** pill (999px)
- **Copy/Run:** transparent bg, 1px emerald border, emerald mono text 12.5px, padding 8px 16px
- **Hover/Focus:** fills emerald, text flips to #0e120f; active presses down 1px; success pops once (scale keyframe)

### Command line
- **Style:** the `$` prompt in emerald, command in mono ink, Copy pill on the right; wraps below 720px
- **Role:** the install command appears here, in the hero and at the close

### Cards / Containers
- **File window:** the only container. Tab bar (chrome, active tab = raised + emerald underline), body (raised, mono artifact text), status line (chrome: ECG svg, state text, tabular cost). 10px radius, 1px hairline, no shadow
- **Annotation row:** no box; a fragment in mono (emerald key) beside a heading and prose, over a top hairline

### Navigation
- None. The page is a single scroll; the footer carries no logo, only the
  tagline and the GitHub link.

### Signature: the run-in line
Lines the run streams back into a file appear as indented comment lines
(gray for tool output, emerald for `✓ goal reached`) and rise in with a
0.5s ease-out. The ECG dash beneath beats while a run is live.

## Do's and Don'ts

### Do:
- **Do** put machine artifacts in file windows and human prose on the ground; the split is the system.
- **Do** use tabular numerals for every cost and use the ECG line wherever a run is shown.
- **Do** keep all motion to opacity and transform, and disable it fully under prefers-reduced-motion.

### Don't:
- **Don't** introduce a second accent hue or a glow; emerald alone carries
  meaning. Text gradients appear only on the hero display, in the emerald
  family, per the owner's accepted exception.
- **Don't** add shadows or nested containers; every box follows the
  file-window grammar (file windows, the three workflow cards), and no
  surface ever gets a box-shadow.
- **Don't** ship a header or sticky nav; the page is one scroll from pitch to install.
