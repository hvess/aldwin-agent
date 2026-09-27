---
name: ux
description: The usability bar for Aldwin's interface, with Apple's Human Interface Guidelines as the only reference for usability and interaction. Use when designing, building, changing or reviewing anything the developer sees or operates — a screen, a key binding, a prompt, a message, a flow. The Aldwin Design System stays the authority for visual values.
---

# UX

Your job is the quality and usability of the application. An interface is
built for humans, not machines, and the bar is this: **someone already knows
how to use it without having to learn it.** If a flow needs explaining, the
flow is wrong; fix the flow, not the explanation.

## The authority

**Apple's Human Interface Guidelines are the only reference for usability
and interaction.** When designing, creating or changing an interface, justify
decisions from the HIG and nothing else — not other design systems, not
other agents or TUIs, not your own taste.

It divides responsibility with the Aldwin Design System (AGENTS.md, "Design
System"):

| Question | Authority |
|---|---|
| Is it intuitive, familiar, predictable, forgiving? What happens on a key, a click, an error? What does the text say? | **HIG** |
| Which colour, glyph, ground, grid position? | **Aldwin Design System** (`docs/design/`, generated into `tokens.rs`) |

Where the two disagree, do not quietly pick one. Record it in
`crates/review/baseline.json` under `contradictions` with both halves and
which the app follows, as every other design disagreement is recorded, and
tell the developer.

Read the HIG pages themselves; do not work from memory of them. The pages are
at `https://developer.apple.com/design/human-interface-guidelines/<page>`,
and their text can be fetched as JSON from
`https://developer.apple.com/tutorials/data/design/human-interface-guidelines/<page>.json`.

## The principles

The HIG's design principles (`design-principles`), each the HIG's own
sentence, and what each means for this app:

- **Familiarity — "Build on what people know."** The most important one
  here. Use the keys and behaviours a terminal user already has in their
  hands: Enter confirms, Esc cancels or backs out, arrows move, Tab moves
  focus, Ctrl-C interrupts. A new binding for something that already has a
  convention is a defect.
- **Simplicity — "Be clear and direct."** One obvious way to do each thing.
  Every element on screen has to earn its place.
- **Agency — "Let people do things their own way."** The developer stays in
  control: nothing irreversible happens without their say, and they can
  always back out.
- **Responsibility — "Act in people's best interest."** Never surprise,
  never lose work, never hide what the agent did.
- **Flexibility — "Adapt to diverse contexts and needs."** Narrow and wide
  terminals, light and dark themes, keyboard and mouse.
- **Purpose — "Make something meaningful."** Each screen exists for one job.
- **Craft — "Care about every detail."**
- **Delight — "Make it human."**

## Pages to read by task

A terminal app has no touch, haptics or app icon; these are the HIG pages
that apply. Read the relevant one before starting the task.

| Task | HIG page |
|---|---|
| Any text on screen: labels, messages, errors, empty states | `writing` |
| Key bindings, shortcuts | `keyboards` |
| Mouse, clicks, drags, selection | `pointing-devices`, `focus-and-selection` |
| Something is running or waiting | `feedback`, `loading` |
| A screen that takes over (the review, a question) | `modality` |
| Anything reversible or destructive | `undo-and-redo` |
| Text entry, the prompt field | `entering-data` |
| Search, pickers, `/resume` | `searching` |
| First launch, the launch card | `launching`, `onboarding` |
| Help, discoverability of commands | `offering-help` |
| Configuration | `settings` |
| Contrast, colour-blindness, keyboard-only use | `accessibility` |
| Spacing, alignment, hierarchy | `layout`, `typography` |

## The gate

Before calling an interface change done, every line below must hold. Report
each as **pass** or the specific failure.

1. **No learning required.** Someone who has never seen this screen knows
   what it is for and what to do next within a glance.
2. **Familiar controls.** Every key and mouse action does what it does in
   other terminal and desktop apps. Anything new is justified by a HIG page.
3. **Always a way out.** Esc (or the documented equivalent) leaves every
   modal state, and leaving loses nothing unexpectedly.
4. **Feedback for everything.** Every action gets a visible response;
   anything that takes time shows that it is running.
5. **Errors say what happened and what to do.** In plain language, per
   `writing` — "Write clear error messages".
6. **Nothing destructive by accident.** Irreversible actions need a
   deliberate step; reversible ones can be undone.
7. **Consistent.** The same thing looks, reads and behaves the same
   everywhere in the app.
8. **Accessible.** Readable in both themes, and meaning never carried by
   colour alone.
9. **Checked by use, not by reading the code.** Run the app (the `run`
   skill) or read the rendered frames from `/review`, and walk the flow as a
   first-time user would.
