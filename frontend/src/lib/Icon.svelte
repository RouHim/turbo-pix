<script>
  import alertTriangle from 'feather-icons/dist/icons/alert-triangle.svg?raw';
  import camera from 'feather-icons/dist/icons/camera.svg?raw';
  import check from 'feather-icons/dist/icons/check.svg?raw';
  import chevronLeft from 'feather-icons/dist/icons/chevron-left.svg?raw';
  import chevronRight from 'feather-icons/dist/icons/chevron-right.svg?raw';
  import download from 'feather-icons/dist/icons/download.svg?raw';
  import heart from 'feather-icons/dist/icons/heart.svg?raw';
  import info from 'feather-icons/dist/icons/info.svg?raw';
  import maximize from 'feather-icons/dist/icons/maximize.svg?raw';
  import menu from 'feather-icons/dist/icons/menu.svg?raw';
  import minimize from 'feather-icons/dist/icons/minimize.svg?raw';
  import minus from 'feather-icons/dist/icons/minus.svg?raw';
  import play from 'feather-icons/dist/icons/play.svg?raw';
  import plus from 'feather-icons/dist/icons/plus.svg?raw';
  import rotateCcw from 'feather-icons/dist/icons/rotate-ccw.svg?raw';
  import rotateCw from 'feather-icons/dist/icons/rotate-cw.svg?raw';
  import search from 'feather-icons/dist/icons/search.svg?raw';
  import x from 'feather-icons/dist/icons/x.svg?raw';

  const ICONS = {
    'alert-triangle': alertTriangle,
    camera,
    check,
    'chevron-left': chevronLeft,
    'chevron-right': chevronRight,
    download,
    heart,
    info,
    maximize,
    menu,
    minimize,
    minus,
    play,
    plus,
    'rotate-ccw': rotateCcw,
    'rotate-cw': rotateCw,
    search,
    x,
  };

  const { name, width = 24, height = 24, class: className = '', ariaHidden = true } = $props();

  const svg = $derived(
    (() => {
      const raw = ICONS[name];
      if (!raw) return '';
      // Strip width/height/class/aria-hidden from the raw tag (browsers keep the
      // FIRST duplicate attribute, so they must be removed before injecting ours).
      return raw.replace(/<svg([^>]*)>/, (_m, attrs) => {
        const clean = attrs.replace(/\s(width|height|class|aria-hidden)="[^"]*"/g, '');
        return `<svg${clean} width="${width}" height="${height}" class="${className}" aria-hidden="${ariaHidden ? 'true' : 'false'}">`;
      });
    })()
  );
</script>

{#if svg}
  <!-- eslint-disable-next-line svelte/no-at-html-tags -- icon names come from internal literals only, never user input -->
  {@html svg}
{/if}
