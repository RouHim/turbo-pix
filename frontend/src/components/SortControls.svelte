<script>
  import { t } from '../lib/i18n.js';
  import { route, pushState } from '../lib/router.svelte.js';

  const options = [
    { value: 'date_desc', key: 'ui.newest_first', fallback: 'Newest First' },
    { value: 'date_asc', key: 'ui.oldest_first', fallback: 'Oldest First' },
    { value: 'name_asc', key: 'ui.name_a_z', fallback: 'Name A-Z' },
    { value: 'name_desc', key: 'ui.name_z_a', fallback: 'Name Z-A' },
    { value: 'size_desc', key: 'ui.largest_first', fallback: 'Largest First' },
    { value: 'size_asc', key: 'ui.smallest_first', fallback: 'Smallest First' },
  ];

  function onChange(e) {
    const sort = e.target.value;
    pushState({ sort });
  }
</script>

<select
  id="sort-select"
  class="sort-select"
  value={route.sort}
  onchange={onChange}
  aria-label={$t('ui.sort_by', { default: 'Sort by' })}
>
  {#each options as opt (opt.value)}
    <option value={opt.value}>{$t(opt.key, { default: opt.fallback })}</option>
  {/each}
</select>

<style>
  .sort-select {
    padding: var(--space-2) var(--space-3);
    border: 1px solid var(--divider-color);
    border-radius: var(--radius-md);
    background: var(--surface-color);
    color: var(--text-primary);
    font-size: var(--font-base);
    font-family: var(--font-body);
    cursor: pointer;
  }

  .sort-select:focus {
    outline: none;
    border-color: var(--primary-color);
  }

  @media (width <= 480px) {
    .sort-select {
      width: 100%;
    }
  }
</style>
