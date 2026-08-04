/**
 * i18n key integrity guard.
 *
 * Scans every i18n key usage in frontend/src against both dictionaries
 * (en + de). Fails listing ALL unresolved keys and any en/de parity drift,
 * so a typo'd or newly-added key can never render raw to users again.
 *
 * Usage: node --test tests/i18n-integrity.test.js  (or npm run test:i18n)
 */

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync, readdirSync } from 'node:fs';
import { join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';

const ROOT = fileURLToPath(new URL('..', import.meta.url));
const SRC_DIR = join(ROOT, 'frontend', 'src');
const I18N_DIR = join(SRC_DIR, 'i18n');
const EN_JSON = join(I18N_DIR, 'en.json');
const DE_JSON = join(I18N_DIR, 'de.json');
const CONSTANTS_JS = join(SRC_DIR, 'lib', 'constants.js');
const INDEXING_ORBIT_SVELTE = join(SRC_DIR, 'components', 'IndexingOrbit.svelte');

/** Recursively collect *.svelte and *.js files under dir, skipping i18n/. */
function walkSources(dir) {
  const files = [];
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    if (entry.isDirectory()) {
      if (entry.name !== 'i18n') files.push(...walkSources(join(dir, entry.name)));
    } else if (entry.isFile() && /\.(svelte|js)$/.test(entry.name)) {
      files.push(join(dir, entry.name));
    }
  }
  return files;
}

/** Flatten a JSON object into dotted leaf paths. */
function flattenPaths(obj, prefix = '') {
  const paths = new Set();
  for (const [key, value] of Object.entries(obj)) {
    const path = prefix ? `${prefix}.${key}` : key;
    if (value && typeof value === 'object' && !Array.isArray(value)) {
      for (const p of flattenPaths(value, path)) paths.add(p);
    } else {
      paths.add(path);
    }
  }
  return paths;
}

/** Parse `const NAME = [ 'a', 'b', ... ];` out of a source file. */
function parseArrayEnum(text, name) {
  const marker = `const ${name} = [`;
  const start = text.indexOf(marker);
  assert.ok(start !== -1, `could not find \`${marker}\` for enum ${name}`);
  const end = text.indexOf(']', start + marker.length);
  return [...text.slice(start + marker.length, end).matchAll(/['"]([^'"]+)['"]/g)].map((m) => m[1]);
}

/** Parse the `id:` fields of the PHASES array in IndexingOrbit.svelte. */
function parsePhaseIds(text) {
  const marker = 'const PHASES = [';
  const start = text.indexOf(marker);
  assert.ok(start !== -1, 'could not find `const PHASES = [` in IndexingOrbit.svelte');
  const end = text.indexOf(']', start + marker.length);
  return [...text.slice(start + marker.length, end).matchAll(/\bid:\s*'([a-z_]+)'/g)].map(
    (m) => m[1]
  );
}

function lineOf(text, index) {
  return text.slice(0, index).split('\n').length;
}

test('i18n key integrity', () => {
  const en = JSON.parse(readFileSync(EN_JSON, 'utf8'));
  const de = JSON.parse(readFileSync(DE_JSON, 'utf8'));
  const enPaths = flattenPaths(en);
  const dePaths = flattenPaths(de);

  const constantsText = readFileSync(CONSTANTS_JS, 'utf8');
  const orbitText = readFileSync(INDEXING_ORBIT_SVELTE, 'utf8');

  // Enumerations for `${…}` placeholders in template keys.
  const enums = {
    'phase.id': parsePhaseIds(orbitText),
    monthKey: parseArrayEnum(constantsText, 'MONTH_KEYS'),
    weekdayKey: parseArrayEnum(constantsText, 'WEEKDAY_KEYS'),
  };

  // These counts are load-bearing: a parse regression (or a new phase/month/
  // weekday added without the dictionaries being updated) must fail loudly
  // instead of vacantly passing the existence check below.
  assert.equal(enums['phase.id'].length, 6, 'PHASES parse drifted — expected 6 ids');
  assert.equal(enums.monthKey.length, 12, 'MONTH_KEYS parse drifted — expected 12 keys');
  assert.equal(enums.weekdayKey.length, 7, 'WEEKDAY_KEYS parse drifted — expected 7 keys');

  /** Record a key usage with its file:line. */
  const usages = [];
  const use = (key, file, line) => usages.push({ key, file, line });

  // 1) Literal keys: $t('key') and get(t)('key').
  const literalPatterns = [/\$t\(\s*['"]([^'"]+)['"]/g, /\bget\(t\)\s*\(\s*['"]([^'"]+)['"]/g];

  // 2) Template keys: $t(`…`) and get(t)(`…`). The alternation is required:
  // a bare `$t(` pattern would also match the `t(` inside `get(t)(`, corrupting
  // the captured template (e.g. `t)(ui.months.${monthKey}`).
  const templatePattern = /\bget\(t\)\s*\(\s*`([^`]+)`|\$t\(\s*`([^`]+)`/g;

  const violations = [];
  for (const file of walkSources(SRC_DIR)) {
    const text = readFileSync(file, 'utf8');
    const rel = relative(ROOT, file);

    for (const pattern of literalPatterns) {
      for (const match of text.matchAll(pattern)) {
        use(match[1], rel, lineOf(text, match.index));
      }
    }

    for (const match of text.matchAll(templatePattern)) {
      const content = match[1] ?? match[2];
      const line = lineOf(text, match.index);
      const parts = content.split(/\$\{([^}]+)\}/g);
      let keys = [''];
      for (let i = 0; i < parts.length; i++) {
        if (i % 2 === 0) {
          keys = keys.map((k) => k + parts[i]);
        } else {
          const expr = parts[i].trim();
          const values = enums[expr];
          if (!values) {
            // Unknown placeholder: a new template site must not silently skip
            // the check — fail and name the site.
            violations.push(
              `unknown template placeholder \${${expr}} in \`${content}\` (${rel}:${line})`
            );
            keys = [];
            break;
          }
          keys = keys.flatMap((k) => values.map((v) => k + v));
        }
      }
      for (const key of keys) use(key, rel, line);
    }

    // 3) Map-defined keys: Sidebar.svelte + SortControls.svelte `key:` fields,
    //    and App.svelte titleKeys values (used via $t(view.key) / $t(opt.key)
    //    / $t(titleKeys[route.view])).
    if (file.endsWith('Sidebar.svelte') || file.endsWith('SortControls.svelte')) {
      for (const match of text.matchAll(/\bkey:\s*'([^']+)'/g)) {
        use(match[1], rel, lineOf(text, match.index));
      }
    }
    if (file.endsWith('App.svelte')) {
      // Only the `titleKeys` object holds i18n keys — `titleFallbacks` right
      // below it holds plain English fallback strings that would false-positive.
      const marker = 'const titleKeys = {';
      const start = text.indexOf(marker);
      assert.ok(start !== -1, 'could not find `const titleKeys = {` in App.svelte');
      const end = text.indexOf('};', start + marker.length);
      const titleKeysText = text.slice(start, end);
      for (const match of titleKeysText.matchAll(
        /^\s*(?:all|favorites|videos|collages|housekeeping):\s*'([^']+)'/gm
      )) {
        use(match[1], rel, lineOf(text, start + match.index));
      }
    }
  }

  // Assertions — collect EVERY violation, do not stop at the first.
  for (const { key, file, line } of usages) {
    if (!enPaths.has(key)) violations.push(`missing from en.json: \`${key}\` (${file}:${line})`);
    if (!dePaths.has(key)) violations.push(`missing from de.json: \`${key}\` (${file}:${line})`);
  }
  for (const key of [...enPaths].filter((k) => !dePaths.has(k))) {
    violations.push(`en/de parity: \`${key}\` only in en.json`);
  }
  for (const key of [...dePaths].filter((k) => !enPaths.has(k))) {
    violations.push(`en/de parity: \`${key}\` only in de.json`);
  }

  const distinct = new Set(usages.map((u) => u.key));

  if (violations.length > 0) {
    console.error('UNRESOLVED I18N KEYS:');
    for (const v of violations) console.error(`  ${v}`);
    process.exitCode = 1;
    assert.fail(`${violations.length} i18n violation(s) — see UNRESOLVED I18N KEYS above`);
  }

  console.log(`i18n integrity: ${distinct.size} keys checked, 0 unresolved, en/de parity OK`);
});
