# Search Animation Fix Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the buggy radar-pulse animation on the search button with a clean spinner inside the search input, eliminating button collapse/layout shift while searching.

**Architecture:** The `searching` state flag (SearchBar.svelte) already tracks the load lifecycle correctly — only the visual is broken. The fix (1) moves the loading indicator from the button into a wrapper around the input field, (2) keeps the button label constant (no text swap), (3) deletes the dead radar CSS from both `app.css` and the scoped SearchBar style, and (4) adds one E2E regression test.

**Tech Stack:** Svelte 5 (runes), Vite, Playwright. No backend changes, no i18n changes (spinner is icon-only, `aria-hidden`).

## Global Constraints

- Frontend changes must run `npm run build` (dist is embedded by `build.rs`) and `npm run lint` / `npm run format:check` before finishing.
- i18n parity: en.json and de.json must stay structurally identical — this change adds no visible text, so no new keys.
- Zero-warnings policy: no unused CSS, no `svelte-ignore` comments (they trip eslint's `svelte/no-unused-svelte-ignore`).
- Reduced motion: the global `@media (prefers-reduced-motion: reduce)` rule in `app.css` already neutralizes animations (`animation-duration: 0.01ms !important`) — the new spinner must NOT opt out of it.
- E2E: `tests/e2e/specs/` run sequentially (workers: 1) against the real backend; the shared model cache in `./data/models` is pre-downloaded, so semantic search works in CI.

---

### Task 1: Move the searching indicator into the input field

**Files:**
- Modify: `frontend/src/components/SearchBar.svelte` (template lines ~325-343, style lines ~396-475)

**Interfaces:**
- Consumes: existing `searching` `$state` flag (SearchBar.svelte:15) — unchanged semantics: `true` while the grid loads, `false` otherwise.
- Produces: `.search-field` wrapper with a `.search-spinner` child; button renders `$t('ui.search', { default: 'Search' })` unconditionally.

- [ ] **Step 1: Wrap the input in a relative container and add the spinner**

Replace the current input markup (SearchBar.svelte:327-339) with:

```svelte
<div class="search-field">
  <input
    type="text"
    id="search-input"
    class="search-input"
    placeholder={$t('ui.search_ai_placeholder', { default: 'AI-powered photo search...' })}
    aria-label={$t('ui.search', { default: 'Search' })}
    bind:value={query}
    bind:this={inputEl}
    onkeydown={onKeydown}
    onfocus={onFocus}
    oninput={onInput}
    onblur={() => {
      focused = false;
      setTimeout(() => (showSuggestions = false), 150);
    }}
  />
  {#if searching}
    <span class="search-spinner" data-testid="search-spinner" aria-hidden="true"></span>
  {/if}
</div>
```

- [ ] **Step 2: Fix the button — remove the text swap and the `searching` class binding**

Replace SearchBar.svelte:341-343 with:

```svelte
<button type="button" id="search-btn" class="search-btn" onclick={submitSearch}>
  {$t('ui.search', { default: 'Search' })}
</button>
```

- [ ] **Step 3: Update the scoped styles**

In `<style>` (SearchBar.svelte):

1. Change `.search-input` (lines ~396-405): add `padding-right: var(--space-6)` (spinner clearance) and remove nothing else:

```css
.search-input {
  flex: 1;
  width: 100%;
  padding: var(--space-3) var(--space-4);
  padding-right: var(--space-6);
  border: 1px solid var(--divider-color);
  border-radius: var(--radius-md);
  font-size: var(--font-lg);
  transition: var(--transition-fast);
  background: var(--background-color);
  color: var(--text-primary);
  font-family: var(--font-body);
}
```

2. Add the wrapper + spinner rules immediately after `.search-input`:

```css
.search-field {
  position: relative;
  flex: 1;
  display: flex;
  align-items: center;
}

.search-spinner {
  position: absolute;
  right: var(--space-3);
  width: 16px;
  height: 16px;
  border: 2px solid var(--divider-color);
  border-top-color: var(--primary-color);
  border-radius: var(--radius-full);
  animation: search-spin 0.8s linear infinite;
  pointer-events: none;
}

@keyframes search-spin {
  to {
    transform: rotate(360deg);
  }
}
```

3. Delete the entire scoped radar block (SearchBar.svelte:452-475): `.search-btn.searching::before/::after`, `.search-btn.searching::after`, `@keyframes radar-pulse`.
4. In `.search-btn` (lines ~414-428), remove the now-dead declarations `position: relative;` and `overflow: visible;` (both existed only for the radar rings; `overflow: visible` is the default anyway).

- [ ] **Step 4: Verify the render in isolation**

Run: `npm run build`
Expected: build succeeds; `dist/assets/index.js` no longer contains the `radar-pulse` keyframe name.

- [ ] **Step 5: Commit**

```bash
git add frontend/src/components/SearchBar.svelte
git commit -m "fix: move search loading indicator from button into input field"
```

---

### Task 2: Delete the dead global radar CSS

**Files:**
- Modify: `frontend/src/app.css:1328-1368`

**Interfaces:**
- Consumes: Task 1 removed all `.searching`-class usage from the template.
- Produces: no global `.search-btn`/`radar-pulse` rules remain anywhere.

- [ ] **Step 1: Remove the radar block**

Delete the entire block from `app.css` (comment `/* Search Button Radar Pulse Animation */` through the closing brace of `@keyframes radar-pulse`):

```css
/* Search Button Radar Pulse Animation */
.search-btn {
  position: relative;
  overflow: visible;
}

.search-btn.searching::before,
.search-btn.searching::after {
  content: '';
  position: absolute;
  top: 50%;
  left: 50%;
  transform: translate(-50%, -50%);
  width: 100%;
  height: 100%;
  border-radius: inherit;
  border: 2px solid var(--primary-color);
  opacity: 0;
  pointer-events: none;
  animation: radar-pulse 1.5s cubic-bezier(0.4, 0, 0.6, 1) infinite;
}

.search-btn.searching::after {
  animation-delay: 0.5s;
}

@keyframes radar-pulse {
  0% {
    transform: translate(-50%, -50%) scale(1);
    opacity: 0.8;
  }

  50% {
    opacity: 0.4;
  }

  100% {
    transform: translate(-50%, -50%) scale(2.5);
    opacity: 0;
  }
}
```

- [ ] **Step 2: Verify no dangling references**

Run: `grep -rn "radar-pulse\|\.searching" frontend/src`
Expected: no matches (or only matches unrelated to search, if any appear, re-check Task 1 Step 3.3 removed the scoped block).

- [ ] **Step 3: Commit**

```bash
git add frontend/src/app.css
git commit -m "fix: remove dead radar-pulse search animation CSS"
```

---

### Task 3: E2E regression test

**Files:**
- Create: `tests/e2e/specs/search-animation.e2e.spec.js`
- Modify: none

**Interfaces:**
- Consumes: `TestHelpers` from `tests/e2e/setup/test-helpers.js` (`performSearch`, `waitForPhotosToLoad`, `selectors.searchInput`, `selectors.searchBtn`), the seeded library (test-data photos are always indexed).
- Produces: `search-animation.e2e.spec.js` — a spec file following the existing `tests/e2e/specs/search.e2e.spec.js` conventions (setupConsoleMonitoring, `test.describe` wrapper if desired).

- [ ] **Step 1: Write the failing test**

```js
import { test, expect } from '@playwright/test';
import { TestHelpers } from '../setup/test-helpers.js';

test.describe('Search animation', () => {
  test.beforeEach(async ({ page }) => {
    TestHelpers.setupConsoleMonitoring(page);
    await TestHelpers.goto(page);
    await TestHelpers.waitForSearchReady(page);
  });

  test('button keeps its label and input shows a spinner while searching', async ({ page }) => {
    // GIVEN: User is on the homepage with a searchable library

    // WHEN: User starts a semantic search (slow: ~3s embedding generation)
    const searchResponse = page.waitForResponse(
      (response) =>
        response.url().includes('/api/search/semantic') && response.status() === 200
    );
    await TestHelpers.performSearch(page, 'cat');
    await searchResponse;

    // THEN: The search button keeps its label (no collapse / layout shift)
    await expect(page.locator(TestHelpers.selectors.searchBtn)).toHaveText('Search');

    // AND: A spinner is visible inside the input while results load
    const spinner = page.locator('[data-testid="search-spinner"]');
    await expect(spinner).toBeVisible();

    // AND: The spinner disappears once results are rendered
    await TestHelpers.waitForPhotosToLoad(page);
    await expect(spinner).not.toBeVisible();
  });
});
```

- [ ] **Step 2: Run the test to verify it fails on current code**

Run: `npx playwright test tests/e2e/specs/search-animation.e2e.spec.js`
Expected (on the pre-fix tree): FAIL — button text is empty while searching (label assertion), or the spinner selector matches nothing.

- [ ] **Step 3: Re-run after Tasks 1-2**

Run: `npx playwright test tests/e2e/specs/search-animation.e2e.spec.js`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add tests/e2e/specs/search-animation.e2e.spec.js
git commit -m "test: search animation keeps button label and shows input spinner"
```

---

### Task 4: Full verification

**Files:** none (verification only).

- [ ] **Step 1: Frontend gates**

```bash
npm run build
npm run lint
npm run format:check
npm test:i18n
```

Expected: all pass (i18n untouched, so parity holds).

- [ ] **Step 2: Backend gates**

Run `cargo build --bin turbo-pix` (build.rs embeds the fresh `dist/`) — expected: success, no warnings.

- [ ] **Step 3: Manual smoke test**

```bash
nohup cargo run > /tmp/tp-server.log 2>&1 &
curl --retry 5 --retry-delay 2 http://localhost:18473/health
```

- Open `http://localhost:18473` in a browser, type a 2+ char query in the search field, wait 300ms+.
- Expected: the button keeps showing "Search" at constant width; a spinner appears at the right edge of the input; the input does not resize during the load; after results render the spinner disappears.
- Also verify with `prefers-reduced-motion: reduce` (devtools → rendering → emulate) that the spinner renders statically (no infinite spin).
- Kill the server afterwards (`pkill -9 -f 'target/(debug|release)/turbo-pix'`).

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "chore: verify search animation fix end to end"
```

---

## Self-Review

- **Spec coverage:** user request (fix buggy/ugly search animation) → Task 1 (indicator moved, no collapse), Task 2 (dead CSS removed), Task 3 (regression test), Task 4 (gates + manual verification). All three root causes addressed: text swap (Task 1.2), ring-on-button visual (Task 1.3/2), duplicate CSS (Task 2).
- **Placeholder scan:** no TBD/TODO; all code blocks are complete.
- **Type consistency:** `searching` flag, `#search-btn`, `#search-input`, `data-testid="search-spinner"` names are consistent across all tasks.
- **Behavioral note:** prefix queries (`type:video`, `location:`, `is_favorite:`) set `searching = false` immediately in `performSearch` and return, so they show no spinner (fast text path) — unchanged behavior, the grid skeleton still covers them.
