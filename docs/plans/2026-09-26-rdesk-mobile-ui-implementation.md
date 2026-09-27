# R-Desk Mobile UI Implementation Plan

**Goal:** Implement the approved three-screen mobile design for the existing R-Desk UI at widths below 768 px.

**Architecture:** Keep desktop routes and components intact. At mobile widths, the root layout renders a mobile shell with a four-item bottom navigation and route-specific mobile content. Both variants use the same device registration, discovery, session launch, history, authentication, and theme services. The existing session route gets a responsive status view; unavailable touch controls remain visibly disabled.

**Tech Stack:** React, React Router, TypeScript, Tailwind/CSS, Vitest, Testing Library.

## Steps

1. Add a failing component test for the mobile shell and navigation while retaining the desktop shell above the breakpoint.
2. Build the mobile shell and responsive routing. Add home, devices, history, and settings views with real data and functional available actions.
3. Add focused tests for remote launch, empty states, and navigation, then implement the responsive session status view.
4. Verify type checking, targeted and full frontend tests, production build, and 390 px visual behavior in a browser.

## Constraints

- Preserve existing uncommitted changes in this working tree.
- Do not invent devices, connection records, status metrics, or working touch controls.
- Session rendering remains owned by the existing native display path; this task implements the mobile UI, not a new mobile transport or renderer.
