<script>
  import { icons } from 'feather-icons';

  const { name, width = 24, height = 24, className = '' } = $props();

  const svg = $derived(icons[name]?.toSvg({ width, height, class: className }) || '');
</script>

{#if svg}
  {@html svg}
{/if}
