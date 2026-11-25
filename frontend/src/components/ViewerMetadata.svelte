<script>
  import { getLocale, t } from '../lib/i18n.js';
  import {
    formatFileSize,
    formatDuration,
    isCollagePhoto,
    isMetadataEditable,
    isVideoFile,
  } from '../lib/utils.js';
  import { formatDateTime } from '../lib/datetime.js';
  import { formatPlaceLabel } from '../lib/map.js';
  import Icon from './Icon.svelte';
  import MiniMap from './MiniMap.svelte';

  const {
    photo = null,
    onEditMetadata = () => {},
    onCloseSidebar = () => {},
    onFilterByPlace = () => {},
    onOpenMap = () => {},
  } = $props();

  const showEditBtn = $derived(photo && !isCollagePhoto(photo) && isMetadataEditable(photo));
  const isVideo = $derived(photo ? isVideoFile(photo.filename) : false);
  const isCollage = $derived(photo ? isCollagePhoto(photo) : false);

  function getFormatName(p) {
    if (!p?.mime_type) return 'file';
    const mimeType = p.mime_type.toLowerCase();
    const map = {
      'image/x-canon-cr2': 'RAW (CR2)',
      'image/x-canon-cr3': 'RAW (CR3)',
      'image/x-nikon-nef': 'RAW (NEF)',
      'image/x-sony-arw': 'RAW (ARW)',
      'image/x-adobe-dng': 'RAW (DNG)',
      'image/x-olympus-orf': 'RAW (ORF)',
      'image/x-panasonic-rw2': 'RAW (RW2)',
      'image/webp': 'WebP',
      'image/heic': 'HEIC',
      'image/heif': 'HEIF',
      'image/avif': 'AVIF',
      'video/mp4': 'video',
      'video/quicktime': 'video',
      'video/x-m4v': 'video',
      // Containers the viewer plays but the metadata endpoint cannot rewrite:
      // the disabled tooltip must name the format, not a vague "video".
      'video/x-matroska': 'Matroska',
      'video/webm': 'WebM',
      'video/x-msvideo': 'AVI',
    };
    return map[mimeType] || mimeType.replace('image/', '').toUpperCase();
  }

  function setField(value) {
    return value || '-';
  }

  function fieldOpacity(value) {
    return value ? '1' : '0.5';
  }

  const metadata = $derived(photo?.metadata || {});
  const camera = $derived(metadata.camera || {});
  const location = $derived(metadata.location || {});
  const settings = $derived(metadata.settings || {});
  const videoMeta = $derived(metadata.video || {});

  const title = $derived.by(
    () =>
      photo?.filename ||
      (photo
        ? `${$t('ui.photo', { default: 'Photo' })} ${photo.hash_sha256?.substring(0, 8)}`
        : '-')
  );
  const dateText = $derived(
    photo?.taken_at
      ? formatDateTime(photo.taken_at, getLocale())
      : $t('ui.unknown', { default: 'Unknown' })
  );
  const sizeText = $derived.by(() => {
    if (!photo) return $t('ui.unknown', { default: 'Unknown' });
    const sz =
      photo.file_size != null
        ? formatFileSize(photo.file_size)
        : $t('ui.unknown', { default: 'Unknown' });
    const dims = photo.width && photo.height ? ` \u2022 ${photo.width}\u00d7${photo.height}` : '';
    return sz + dims;
  });
  const cameraText = $derived(
    camera.make && camera.model
      ? `${camera.make} ${camera.model}`
      : $t('ui.unknown', { default: 'Unknown' })
  );
  const hasCamera = $derived(camera.make || camera.model || camera.lens_make || camera.lens_model);
  // `flash_used` is a boolean, so its presence cannot be read as truthiness —
  // but the scanner now states every settings member explicitly and writes
  // `null` for one the file does not have, so "present and not null" is the
  // check that means "the file carries a flash verdict".
  const hasFlash = $derived(settings.flash_used != null);
  const hasSettings = $derived(
    settings.iso ||
      settings.aperture ||
      settings.shutter_speed ||
      settings.focal_length ||
      settings.exposure_mode ||
      settings.metering_mode ||
      settings.white_balance ||
      hasFlash ||
      photo?.orientation ||
      settings.color_space
  );
  const hasLocation = $derived(
    location.latitude != null || location.longitude != null || Boolean(placeLabel)
  );
  const placeLabel = $derived(formatPlaceLabel(photo, getLocale()));
  const hasCoordinates = $derived(location.latitude != null && location.longitude != null);
  const placeFilterLabel = $derived(
    $t('ui.metadata.place_filter', {
      values: { place: placeLabel ?? '' },
      default: 'Show all photos at {place}',
    })
  );
  // ONE message for the tooltip and the accessible name. A screen reader on a
  // disabled button announces the `aria-label`, not the `title`, so a
  // format-specific `title` beside a generic `aria-label` would hide the
  // format from assistive tech and make the two disagree.
  const editDisabledLabel = $derived.by(() =>
    photo?.mime_type
      ? $t('ui.metadata.edit_unsupported_format', {
          values: { format: getFormatName(photo) },
          default: 'Editing {format} files is not supported',
        })
      : $t('ui.metadata.edit_unsupported', {
          default: 'Editing this file type is not supported',
        })
  );
  const editLabel = $derived(
    showEditBtn ? $t('ui.metadata.edit_button', { default: 'Edit Metadata' }) : editDisabledLabel
  );
</script>

<div class="photo-info">
  <div class="photo-info-header">
    <h3 id="photo-title">{title}</h3>
    <div style="display: flex; gap: var(--space-2); align-items: center;">
      {#if !isCollage}
        <!-- A `disabled` button cannot show its own tooltip: it takes no
             pointer events and, being disabled, no focus, so the format it
             cannot edit never reaches the user. The wrapper is never disabled
             and carries the same reason, which is the one surface that can
             still surface it. -->
        <span class="edit-btn-wrap" title={showEditBtn ? undefined : editLabel}>
          <button
            type="button"
            id="metadata-edit-btn"
            class="btn-icon"
            disabled={!showEditBtn}
            title={editLabel}
            aria-label={editLabel}
            onclick={onEditMetadata}
          >
            <Icon name="edit-2" width={16} height={16} />
          </button>
        </span>
      {/if}
      <button
        type="button"
        id="sidebar-close-btn"
        class="btn-icon"
        title={$t('ui.close', { default: 'Close' })}
        aria-label={$t('ui.close', { default: 'Close' })}
        onclick={onCloseSidebar}
      >
        <Icon name="x" width={16} height={16} />
      </button>
    </div>
  </div>

  <div class="photo-meta">
    <div class="meta-item">
      <span class="meta-label">{$t('ui.date', { default: 'Date:' })}</span>
      <span id="photo-date">{dateText}</span>
    </div>
    <div class="meta-item">
      <span class="meta-label">{$t('ui.size', { default: 'Size:' })}</span>
      <span id="photo-size">{sizeText}</span>
    </div>
    <div class="meta-item">
      <span class="meta-label">{$t('ui.camera', { default: 'Camera:' })}</span>
      <span id="photo-camera">{cameraText}</span>
    </div>
  </div>

  <div class="photo-meta-full">
    <div class="meta-section">
      <h4 class="meta-section-title">
        {$t('ui.metadata.file_information', { default: 'File Information' })}
      </h4>
      <div class="meta-item meta-item-path">
        <span class="meta-label">{$t('ui.metadata.file_path', { default: 'File path:' })}</span
        ><span
          id="meta-filename"
          style="opacity: {fieldOpacity(photo?.file_path || photo?.filename)}"
          >{setField(photo?.file_path || photo?.filename)}</span
        >
      </div>
      <div class="meta-item">
        <span class="meta-label">{$t('ui.metadata.file_size', { default: 'File Size:' })}</span
        ><span id="meta-filesize" style="opacity: {fieldOpacity(photo?.file_size)}"
          >{setField(photo?.file_size != null ? formatFileSize(photo.file_size) : null)}</span
        >
      </div>
      <div class="meta-item">
        <span class="meta-label">{$t('ui.metadata.dimensions', { default: 'Dimensions:' })}</span
        ><span id="meta-dimensions" style="opacity: {fieldOpacity(photo?.width && photo?.height)}"
          >{setField(
            photo?.width && photo?.height ? `${photo.width} \u00d7 ${photo.height} px` : null
          )}</span
        >
      </div>
      <div class="meta-item">
        <span class="meta-label">{$t('ui.metadata.type', { default: 'Type:' })}</span><span
          id="meta-type"
          style="opacity: {fieldOpacity(photo?.mime_type)}">{setField(photo?.mime_type)}</span
        >
      </div>
      <div class="meta-item">
        <span class="meta-label"
          >{$t('ui.metadata.date_modified', { default: 'Date Modified:' })}</span
        ><span id="meta-date-modified" style="opacity: {fieldOpacity(photo?.date_modified)}"
          >{setField(
            photo?.date_modified ? formatDateTime(photo.date_modified, getLocale()) : null
          )}</span
        >
      </div>
    </div>

    {#if hasCamera}
      <div class="meta-section" id="camera-section">
        <h4 class="meta-section-title">
          {$t('ui.metadata.camera_section', { default: 'Camera' })}
        </h4>
        <div class="meta-item">
          <span class="meta-label">{$t('ui.metadata.make', { default: 'Make:' })}</span><span
            id="meta-camera-make"
            style="opacity: {fieldOpacity(camera.make)}">{setField(camera.make)}</span
          >
        </div>
        <div class="meta-item">
          <span class="meta-label">{$t('ui.metadata.model', { default: 'Model:' })}</span><span
            id="meta-camera-model"
            style="opacity: {fieldOpacity(camera.model)}">{setField(camera.model)}</span
          >
        </div>
        <div class="meta-item">
          <span class="meta-label">{$t('ui.metadata.lens_make', { default: 'Lens Make:' })}</span
          ><span id="meta-lens-make" style="opacity: {fieldOpacity(camera.lens_make)}"
            >{setField(camera.lens_make)}</span
          >
        </div>
        <div class="meta-item">
          <span class="meta-label">{$t('ui.metadata.lens_model', { default: 'Lens Model:' })}</span
          ><span id="meta-lens-model" style="opacity: {fieldOpacity(camera.lens_model)}"
            >{setField(camera.lens_model)}</span
          >
        </div>
      </div>
    {/if}

    {#if hasSettings}
      <div class="meta-section" id="settings-section">
        <h4 class="meta-section-title">
          {$t('ui.metadata.camera_settings', { default: 'Camera Settings' })}
        </h4>
        <div class="meta-item">
          <span class="meta-label">{$t('ui.metadata.iso', { default: 'ISO:' })}</span><span
            id="meta-iso"
            style="opacity: {fieldOpacity(settings.iso)}"
            >{setField(settings.iso ? `ISO ${settings.iso}` : null)}</span
          >
        </div>
        <div class="meta-item">
          <span class="meta-label">{$t('ui.metadata.aperture', { default: 'Aperture:' })}</span
          ><span id="meta-aperture" style="opacity: {fieldOpacity(settings.aperture)}"
            >{setField(settings.aperture ? `f/${settings.aperture.toFixed(1)}` : null)}</span
          >
        </div>
        <div class="meta-item">
          <span class="meta-label"
            >{$t('ui.metadata.shutter_speed', { default: 'Shutter Speed:' })}</span
          ><span id="meta-shutter" style="opacity: {fieldOpacity(settings.shutter_speed)}"
            >{setField(settings.shutter_speed)}</span
          >
        </div>
        <div class="meta-item">
          <span class="meta-label"
            >{$t('ui.metadata.focal_length', { default: 'Focal Length:' })}</span
          ><span id="meta-focal" style="opacity: {fieldOpacity(settings.focal_length)}"
            >{setField(
              settings.focal_length ? `${settings.focal_length.toFixed(0)} mm` : null
            )}</span
          >
        </div>
        <div class="meta-item">
          <span class="meta-label"
            >{$t('ui.metadata.exposure_mode', { default: 'Exposure Mode:' })}</span
          ><span id="meta-exposure" style="opacity: {fieldOpacity(settings.exposure_mode)}"
            >{setField(settings.exposure_mode)}</span
          >
        </div>
        <div class="meta-item">
          <span class="meta-label"
            >{$t('ui.metadata.metering_mode', { default: 'Metering Mode:' })}</span
          ><span id="meta-metering" style="opacity: {fieldOpacity(settings.metering_mode)}"
            >{setField(settings.metering_mode)}</span
          >
        </div>
        <div class="meta-item">
          <span class="meta-label"
            >{$t('ui.metadata.white_balance', { default: 'White Balance:' })}</span
          ><span id="meta-wb" style="opacity: {fieldOpacity(settings.white_balance)}"
            >{setField(settings.white_balance)}</span
          >
        </div>
        <div class="meta-item">
          <span class="meta-label">{$t('ui.metadata.flash', { default: 'Flash:' })}</span><span
            id="meta-flash"
            style="opacity: {fieldOpacity(hasFlash)}"
            >{setField(
              hasFlash
                ? settings.flash_used
                  ? $t('ui.yes', { default: 'Yes' })
                  : $t('ui.no', { default: 'No' })
                : null
            )}</span
          >
        </div>
        <div class="meta-item">
          <span class="meta-label"
            >{$t('ui.metadata.orientation', { default: 'Orientation:' })}</span
          ><span id="meta-orientation" style="opacity: {fieldOpacity(photo?.orientation)}"
            >{setField(photo?.orientation)}</span
          >
        </div>
        <div class="meta-item">
          <span class="meta-label"
            >{$t('ui.metadata.color_space', { default: 'Color Space:' })}</span
          ><span id="meta-colorspace" style="opacity: {fieldOpacity(settings.color_space)}"
            >{setField(settings.color_space)}</span
          >
        </div>
      </div>
    {/if}

    {#if hasLocation}
      <div class="meta-section" id="location-section">
        <h4 class="meta-section-title">
          {$t('ui.metadata.location_section', { default: 'Location' })}
        </h4>
        {#if placeLabel}
          <div class="meta-item">
            <span class="meta-label">{$t('ui.metadata.place', { default: 'Place:' })}</span>
            <button
              type="button"
              class="meta-place"
              id="meta-place"
              data-testid="meta-place"
              title={placeFilterLabel}
              aria-label={placeFilterLabel}
              onclick={() => onFilterByPlace(location.place)}
            >
              {placeLabel}
            </button>
          </div>
        {/if}
        <div class="meta-item">
          <span class="meta-label">{$t('ui.metadata.gps', { default: 'GPS:' })}</span><span
            id="meta-gps"
            style="opacity: {fieldOpacity(location.latitude != null && location.longitude != null)}"
            >{setField(
              location.latitude != null && location.longitude != null
                ? `${location.latitude.toFixed(6)}, ${location.longitude.toFixed(6)}`
                : null
            )}</span
          >
        </div>
        {#if hasCoordinates}
          <MiniMap latitude={location.latitude} longitude={location.longitude} {onOpenMap} />
        {/if}
      </div>
    {/if}

    {#if isVideo}
      <div class="meta-section" id="video-section">
        <h4 class="meta-section-title">
          {$t('ui.metadata.video_section', { default: 'Video Information' })}
        </h4>
        <div class="meta-item">
          <span class="meta-label">{$t('ui.metadata.duration', { default: 'Duration:' })}</span
          ><span id="meta-duration" style="opacity: {fieldOpacity(photo?.duration)}"
            >{setField(photo?.duration ? formatDuration(photo.duration) : null)}</span
          >
        </div>
        <div class="meta-item">
          <span class="meta-label"
            >{$t('ui.metadata.video_codec', { default: 'Video Codec:' })}</span
          ><span id="meta-video-codec" style="opacity: {fieldOpacity(videoMeta.codec)}"
            >{setField(videoMeta.codec)}</span
          >
        </div>
        <div class="meta-item">
          <span class="meta-label"
            >{$t('ui.metadata.audio_codec', { default: 'Audio Codec:' })}</span
          ><span id="meta-audio-codec" style="opacity: {fieldOpacity(videoMeta.audio_codec)}"
            >{setField(videoMeta.audio_codec)}</span
          >
        </div>
        <div class="meta-item">
          <span class="meta-label">{$t('ui.metadata.frame_rate', { default: 'Frame Rate:' })}</span
          ><span id="meta-framerate" style="opacity: {fieldOpacity(videoMeta.frame_rate)}"
            >{setField(
              videoMeta.frame_rate ? `${videoMeta.frame_rate.toFixed(2)} fps` : null
            )}</span
          >
        </div>
        <div class="meta-item">
          <span class="meta-label">{$t('ui.metadata.bitrate', { default: 'Bitrate:' })}</span><span
            id="meta-bitrate"
            style="opacity: {fieldOpacity(videoMeta.bitrate)}"
            >{setField(videoMeta.bitrate ? `${videoMeta.bitrate} kbps` : null)}</span
          >
        </div>
      </div>
    {/if}
  </div>
</div>

<style>
  .photo-info {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-height: 0;
    overflow-y: auto;
    padding-right: var(--space-3);
    scrollbar-width: thin;
    scrollbar-color: var(--text-muted) transparent;
  }

  .photo-info > :global(div:first-child) {
    margin-top: var(--space-12);
  }

  .photo-info h3 {
    font-size: var(--font-3xl);
    font-weight: var(--font-bold);
    margin: 0 0 var(--space-6);
    color: var(--text-primary);
    line-height: var(--leading-tight);
    letter-spacing: -0.02em;
    /* A filename carries no break opportunity, and the header's buttons leave
       the title only part of the panel: with `break-word` the title box shrank
       to its min-content width but the text still painted past it, because
       `break-word` does not affect intrinsic sizing. `anywhere` does, so the
       name wraps inside the box instead of being clipped at the panel edge. */
    overflow-wrap: anywhere;
  }

  @media (width <= 768px) {
    /* The visible sticky handle replaces the old invisible 89 px top band;
       the header stays inside the 56 px grab zone without floating. */
    .photo-info > :global(div:first-child) {
      margin-top: 0;
    }
  }

  @media (width <= 480px) {
    .photo-info h3 {
      font-size: var(--font-2xl);
      margin-bottom: var(--space-4);
    }
  }

  .photo-info-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    /* The mobile sheet's drag starts here; the browser must not consume the
       vertical gesture as sheet scrolling first. Buttons still click. */
    touch-action: none;
  }

  .photo-meta {
    margin-bottom: var(--space-6);
    padding: var(--space-4);
    background: var(--background-secondary);
    border-radius: var(--radius-md);
    border: 1px solid var(--divider-color);
  }

  .photo-meta-full {
    margin-top: var(--space-4);
    display: block;
  }

  .meta-section {
    margin-bottom: var(--space-8);
    padding: var(--space-5);
    background: var(--background-secondary);
    border-radius: var(--radius-md);
    border: 1px solid var(--divider-color);
  }

  .meta-section:last-child {
    margin-bottom: 0;
  }

  .meta-section-title {
    font-size: var(--font-lg);
    font-weight: var(--font-bold);
    text-transform: uppercase;
    letter-spacing: 0.8px;
    color: var(--primary-ink);
    margin: 0 0 var(--space-4);
    padding-bottom: var(--space-3);
    border-bottom: 1px solid var(--divider-color);
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }

  .photo-meta .meta-item {
    display: flex;
    justify-content: space-between;
    margin-bottom: var(--space-3);
    padding-bottom: var(--space-3);
    border-bottom: 1px solid var(--divider-color);
  }

  .photo-meta-full .meta-item {
    display: flex;
    justify-content: space-between;
    gap: var(--space-4);
    margin-bottom: var(--space-3);
    padding: var(--space-2) 0;
    align-items: center;
    border-bottom: 1px solid var(--divider-color);
  }

  .photo-meta .meta-item:last-child {
    border-bottom: none;
    margin-bottom: 0;
    padding-bottom: 0;
  }

  .photo-meta-full .meta-item:last-child {
    border-bottom: none;
    margin-bottom: 0;
    padding-bottom: 0;
  }

  .photo-meta .meta-item :global(.meta-label) {
    font-weight: var(--font-semibold);
    font-size: var(--font-base);
    color: var(--text-secondary);
  }

  .photo-meta .meta-item :global(span) {
    color: var(--text-primary);
    font-size: var(--font-base);
    font-weight: var(--font-medium);
  }

  .photo-meta-full .meta-item :global(.meta-label) {
    font-weight: var(--font-semibold);
    font-size: var(--font-base);
    color: var(--text-secondary);
    overflow-wrap: break-word;
    min-width: 140px;
    flex-shrink: 0;
  }

  .photo-meta-full .meta-item :global(span) {
    color: var(--text-primary);
    font-size: var(--font-base);
    font-weight: var(--font-medium);
    font-variant-numeric: tabular-nums;
    overflow-wrap: break-word;
    /* A flex item may not shrink below its min-content width, and a file path
       has no break opportunity to lower it — so the value box grew past the
       panel and the path was clipped. `min-width: 0` lets it shrink to the
       column, where the `overflow-wrap` above wraps the path onto more lines. */
    min-width: 0;
    text-align: right;
    flex: 1;
  }

  /* The place name is a filter, not a value: a ghost button (surface fill,
     hairline, 4px radius) is the design system's clickable primitive, and a
     persistent border is the affordance a hover-only underline never gives a
     touch user. */
  .meta-place {
    padding: var(--space-2) var(--space-3);
    border: 1px solid var(--divider-color);
    border-radius: var(--radius-sm);
    background: var(--surface-color);
    color: var(--primary-ink);
    font: inherit;
    font-size: var(--font-base);
    font-weight: var(--font-medium);
    text-align: right;
    overflow-wrap: break-word;
    min-width: 0;
    flex: 1;
    cursor: pointer;
    transition: var(--transition-fast);
  }

  .meta-place:hover {
    border-color: var(--primary-color);
  }

  .meta-place:focus-visible {
    outline: 2px solid var(--primary-color);
    outline-offset: 2px;
  }

  @media (pointer: coarse) {
    .meta-place {
      min-height: 44px;
    }
  }

  #meta-gps {
    font-variant-numeric: tabular-nums;
  }

  .btn-icon {
    background: none;
    border: none;
    cursor: pointer;
    padding: 8px;
    min-width: var(--button-size);
    min-height: var(--button-size);
    border-radius: var(--radius-sm);
    color: var(--text-muted);
    transition: var(--transition-fast);
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .btn-icon:hover {
    background: var(--background-secondary);
    color: var(--text-primary);
  }

  .btn-icon:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }

  .btn-icon:disabled:hover {
    background: none;
    color: var(--text-muted);
  }

  @media (width <= 480px) {
    .photo-meta-full .meta-item-path {
      display: block;
    }

    .photo-meta-full .meta-item-path :global(.meta-label) {
      display: block;
      min-width: 0;
      margin-bottom: var(--space-1);
    }

    .photo-meta-full .meta-item-path :global(span) {
      display: block;
      min-width: 0;
      text-align: left;
      overflow-wrap: anywhere;
    }
  }

  /* The hover surface for a disabled control. `inline-flex` keeps the wrapper's
     box identical to the button's, so the header's spacing does not move. */
  .edit-btn-wrap {
    display: inline-flex;
  }
</style>
