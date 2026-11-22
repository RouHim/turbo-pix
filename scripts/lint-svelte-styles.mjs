import { readFileSync, readdirSync, statSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import stylelint from 'stylelint';

// Scoped `<style>` blocks never reach `stylelint "**/*.css"`. Use a curated
// correctness set (formatting is Prettier's job) so a real CSS mistake inside a
// Svelte component fails the same `npm run lint` gate.
const ROOT = fileURLToPath(new URL('..', import.meta.url));
const SOURCE_DIR = path.join(ROOT, 'frontend/src');

const RULES = {
  'annotation-no-unknown': true,
  'at-rule-no-unknown': true,
  'block-no-empty': true,
  'color-no-invalid-hex': true,
  'comment-no-empty': true,
  'custom-property-no-missing-var-function': true,
  'declaration-block-no-duplicate-custom-properties': true,
  'declaration-block-no-duplicate-properties': [
    true,
    { ignore: ['consecutive-duplicates-with-different-values'] },
  ],
  'declaration-block-no-shorthand-property-overrides': true,
  'font-family-no-duplicate-names': true,
  'font-family-no-missing-generic-family-keyword': true,
  'function-calc-no-unspaced-operator': true,
  'function-linear-gradient-no-nonstandard-direction': true,
  'function-no-unknown': true,
  'keyframe-block-no-duplicate-selectors': true,
  'keyframe-declaration-no-important': true,
  'media-feature-name-no-unknown': true,
  'named-grid-areas-no-invalid': true,
  'no-duplicate-at-import-rules': true,
  'no-duplicate-selectors': true,
  'no-invalid-double-slash-comments': true,
  'no-invalid-position-at-import-rule': true,
  'property-no-unknown': [true, { ignoreProperties: [/^--/] }],
  'selector-anb-no-unmatchable': true,
  'selector-pseudo-class-no-unknown': [true, { ignorePseudoClasses: ['global'] }],
  'selector-pseudo-element-no-unknown': [
    true,
    {
      ignorePseudoElements: [
        'view-transition',
        'view-transition-old',
        'view-transition-new',
        'view-transition-group',
        'view-transition-image-pair',
      ],
    },
  ],
  'string-no-newline': true,
  'unit-no-unknown': true,
};

function walk(directory) {
  return readdirSync(directory).flatMap((name) => {
    const fullPath = path.join(directory, name);
    if (statSync(fullPath).isDirectory()) return walk(fullPath);
    return fullPath.endsWith('.svelte') ? [fullPath] : [];
  });
}

let findings = 0;
for (const file of walk(SOURCE_DIR)) {
  const source = readFileSync(file, 'utf8');
  const styles = [...source.matchAll(/<style(?:\s[^>]*)?>([\s\S]*?)<\/style>/g)];
  for (const [index, match] of styles.entries()) {
    const code = match[1].trim();
    if (!code) continue;

    const result = await stylelint.lint({
      code,
      codeFilename: file,
      config: { rules: RULES },
      configBasedir: ROOT,
    });
    for (const resultFile of result.results) {
      for (const warning of resultFile.warnings) {
        findings += 1;
        const styleLabel = styles.length > 1 ? `#style${index + 1}` : '';
        console.error(
          `${path.relative(ROOT, file)}${styleLabel}:${warning.line}:${warning.column} ` +
            `${warning.rule}: ${warning.text}`
        );
      }
    }
  }
}

if (findings > 0) {
  console.error(`\n${findings} Svelte style issue(s) found.`);
  process.exitCode = 1;
}
