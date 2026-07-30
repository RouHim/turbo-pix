<script>
  import { icons } from 'feather-icons';

  let { name, width = 24, height = 24, className = '' } = $props();

  let svg = $derived(icons[name]?.toSvg({ width, height, class: className }) || '');
</script>

{#if svg}
  {@html svg}
{/if}
