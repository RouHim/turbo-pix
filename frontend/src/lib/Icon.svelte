<script>
  import { icons } from 'feather-icons';

  const { name, width = 24, height = 24, class: className = '', ariaHidden = true } = $props();

  const svg = $derived(
    icons[name]?.toSvg({
      width,
      height,
      class: className,
      'aria-hidden': ariaHidden ? 'true' : 'false',
    }) || ''
  );
</script>

{#if svg}
  <!-- eslint-disable-next-line svelte/no-at-html-tags -- icon names come from internal literals only, never user input -->
  {@html svg}
{/if}
