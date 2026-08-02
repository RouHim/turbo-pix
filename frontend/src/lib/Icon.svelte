<script>
  import alertCircle from 'feather-icons/dist/icons/alert-circle.svg?raw';
  import alertTriangle from 'feather-icons/dist/icons/alert-triangle.svg?raw';
  import calendar from 'feather-icons/dist/icons/calendar.svg?raw';
  import camera from 'feather-icons/dist/icons/camera.svg?raw';
  import check from 'feather-icons/dist/icons/check.svg?raw';
  import checkCircle from 'feather-icons/dist/icons/check-circle.svg?raw';
  import chevronLeft from 'feather-icons/dist/icons/chevron-left.svg?raw';
  import chevronRight from 'feather-icons/dist/icons/chevron-right.svg?raw';
  import clock from 'feather-icons/dist/icons/clock.svg?raw';
  import cpu from 'feather-icons/dist/icons/cpu.svg?raw';
  import download from 'feather-icons/dist/icons/download.svg?raw';
  import edit2 from 'feather-icons/dist/icons/edit-2.svg?raw';
  import fileText from 'feather-icons/dist/icons/file-text.svg?raw';
  import grid from 'feather-icons/dist/icons/grid.svg?raw';
  import heart from 'feather-icons/dist/icons/heart.svg?raw';
  import image from 'feather-icons/dist/icons/image.svg?raw';
  import info from 'feather-icons/dist/icons/info.svg?raw';
  import loader from 'feather-icons/dist/icons/loader.svg?raw';
  import mapPin from 'feather-icons/dist/icons/map-pin.svg?raw';
  import maximize from 'feather-icons/dist/icons/maximize.svg?raw';
  import menu from 'feather-icons/dist/icons/menu.svg?raw';
  import minimize from 'feather-icons/dist/icons/minimize.svg?raw';
  import minus from 'feather-icons/dist/icons/minus.svg?raw';
  import moon from 'feather-icons/dist/icons/moon.svg?raw';
  import play from 'feather-icons/dist/icons/play.svg?raw';
  import plus from 'feather-icons/dist/icons/plus.svg?raw';
  import rotateCcw from 'feather-icons/dist/icons/rotate-ccw.svg?raw';
  import rotateCw from 'feather-icons/dist/icons/rotate-cw.svg?raw';
  import search from 'feather-icons/dist/icons/search.svg?raw';
  import sun from 'feather-icons/dist/icons/sun.svg?raw';
  import trash2 from 'feather-icons/dist/icons/trash-2.svg?raw';
  import video from 'feather-icons/dist/icons/video.svg?raw';
  import x from 'feather-icons/dist/icons/x.svg?raw';

  const ICONS = {
    'alert-circle': alertCircle,
    'alert-triangle': alertTriangle,
    calendar,
    camera,
    check,
    'check-circle': checkCircle,
    'chevron-left': chevronLeft,
    'chevron-right': chevronRight,
    clock,
    cpu,
    download,
    'edit-2': edit2,
    'file-text': fileText,
    grid,
    heart,
    image,
    info,
    loader,
    'map-pin': mapPin,
    maximize,
    menu,
    minimize,
    minus,
    moon,
    play,
    plus,
    'rotate-ccw': rotateCcw,
    'rotate-cw': rotateCw,
    search,
    sun,
    'trash-2': trash2,
    video,
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
