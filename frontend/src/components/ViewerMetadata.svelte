<script>
  import { api } from '../lib/api.js';
  import { t } from '../lib/i18n.js';
  import { formatDate, formatFileSize } from '../lib/utils.js';
  import { APP_CONSTANTS } from '../lib/constants.js';
  import Icon from '../lib/Icon.svelte';

  let {
    photo = null,
    onEditMetadata = () => {},
    showSidebar = false,
    onCloseSidebar = () => {},
  } = $props();

  let showEditBtn = $derived(photo && !isCollagePhoto(photo) && isFormatSupported(photo));
  let isVideo = $derived(photo ? isVideoFile(photo.filename) : false);
  let isCollage = $derived(photo ? isCollagePhoto(photo) : false);

  function isVideoFile(filename) {
    if (!filename) return false;
    const ext = filename.toLowerCase().substring(filename.lastIndexOf('.'));
    return APP_CONSTANTS.VIDEO_EXTENSIONS.includes(ext);
  }

  function isCollagePhoto(photo) {
    return Boolean(photo?.isCollage || photo?.collageId != null);
  }

  function isFormatSupported(p) {
    if (!p?.mime_type) return false;
    const supported = ['image/jpeg', 'image/jpg', 'image/png'];
    return supported.includes(p.mime_type.toLowerCase());
  }

  function getFormatName(p) {
    if (!p?.mime_type) return 'this';
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
      'video/x-msvideo': 'video',
    };
    return map[mimeType] || mimeType.replace('image/', '').toUpperCase();
  }

  function formatDuration(seconds) {
    const hours = Math.floor(seconds / 3600);
    const minutes = Math.floor((seconds % 3600) / 60);
    const secs = Math.floor(seconds % 60);
    if (hours > 0)
      return `${hours}:${String(minutes).padStart(2, '0')}:${String(secs).padStart(2, '0')}`;
    return `${minutes}:${String(secs).padStart(2, '0')}`;
  }

  function setField(id, value) {
    return value || '-';
  }

  function fieldOpacity(value) {
    return value ? '1' : '0.5';
  }

  let metadata = $derived(photo?.metadata || {});
  let camera = $derived(metadata.camera || {});
  let location = $derived(metadata.location || {});
  let settings = $derived(metadata.settings || {});
  let videoMeta = $derived(metadata.video || {});

  let title = $derived(
    photo?.filename || (photo ? `Photo ${photo.hash_sha256?.substring(0, 8)}` : '-')
  );
  let dateText = $derived(photo?.taken_at ? formatDate(photo.taken_at) : '-');
  let sizeText = $derived(() => {
    if (!photo) return '-';
    const sz = photo.file_size ? formatFileSize(photo.file_size) : '-';
    const dims = photo.width && photo.height ? ` \u2022 ${photo.width}\u00d7${photo.height}` : '';
    return sz + dims;
  });
  let cameraText = $derived(camera.make && camera.model ? `${camera.make} ${camera.model}` : '-');
  let locationText = $derived(
    location.latitude && location.longitude
      ? `${location.latitude.toFixed(6)}, ${location.longitude.toFixed(6)}`
      : '-'
  );

  let hasCamera = $derived(camera.make || camera.model || camera.lens_make || camera.lens_model);
  let hasSettings = $derived(
    settings.iso ||
      settings.aperture ||
      settings.shutter_speed ||
      settings.focal_length ||
      settings.exposure_mode ||
      settings.metering_mode ||
      settings.white_balance ||
      settings.flash_used !== undefined ||
      photo?.orientation ||
      settings.color_space
  );
  let hasLocation = $derived(location.latitude || location.longitude);
</script>

<div class="photo-info">
  <div class="photo-info-header">
    <h3 id="photo-title">{title}</h3>
    <div style="display: flex; gap: var(--space-2); align-items: center;">
      {#if showEditBtn && !isCollage}
        <button
          type="button"
          id="metadata-edit-btn"
          class="btn-icon"
          title={$t('ui.metadata.edit_btn', { default: 'Edit Metadata' })}
          onclick={onEditMetadata}
        >
          <Icon name="edit-2" width={16} height={16} />
        </button>
      {/if}
      <button
        type="button"
        id="sidebar-close-btn"
        class="btn-icon"
        title={$t('ui.close', { default: 'Close' })}
        onclick={onCloseSidebar}
      >
        <Icon name="x" width={16} height={16} />
      </button>
    </div>
  </div>

  <div class="photo-meta">
    <div class="meta-item">
      <label>{$t('ui.date', { default: 'Date:' })}</label>
      <span id="photo-date">{dateText}</span>
    </div>
    <div class="meta-item">
      <label>{$t('ui.size', { default: 'Size:' })}</label>
      <span id="photo-size">{sizeText}</span>
    </div>
    <div class="meta-item">
      <label>{$t('ui.camera', { default: 'Camera:' })}</label>
      <span id="photo-camera">{cameraText}</span>
    </div>
    <div class="meta-item">
      <label>{$t('ui.location', { default: 'Location:' })}</label>
      <span id="photo-location">{locationText}</span>
    </div>
  </div>

  <div class="photo-meta-full">
    <div class="meta-section">
      <h4 class="meta-section-title">
        {$t('ui.metadata.file_information', { default: 'File Information' })}
      </h4>
      <div class="meta-item">
        <label>{$t('ui.metadata.file_path', { default: 'File path:' })}</label><span
          id="meta-filename"
          style="opacity: {fieldOpacity(photo?.file_path || photo?.filename)}"
          >{setField('meta-filename', photo?.file_path || photo?.filename)}</span
        >
      </div>
      <div class="meta-item">
        <label>{$t('ui.metadata.file_size', { default: 'File Size:' })}</label><span
          id="meta-filesize"
          style="opacity: {fieldOpacity(photo?.file_size)}"
          >{setField(
            'meta-filesize',
            photo?.file_size ? formatFileSize(photo.file_size) : null
          )}</span
        >
      </div>
      <div class="meta-item">
        <label>{$t('ui.metadata.dimensions', { default: 'Dimensions:' })}</label><span
          id="meta-dimensions"
          style="opacity: {fieldOpacity(photo?.width && photo?.height)}"
          >{setField(
            'meta-dimensions',
            photo?.width && photo?.height ? `${photo.width} \u00d7 ${photo.height} px` : null
          )}</span
        >
      </div>
      <div class="meta-item">
        <label>{$t('ui.metadata.type', { default: 'Type:' })}</label><span
          id="meta-type"
          style="opacity: {fieldOpacity(photo?.mime_type)}"
          >{setField('meta-type', photo?.mime_type)}</span
        >
      </div>
      <div class="meta-item">
        <label>{$t('ui.metadata.date_taken', { default: 'Date Taken:' })}</label><span
          id="meta-date-taken"
          style="opacity: {fieldOpacity(photo?.taken_at)}"
          >{setField('meta-date-taken', photo?.taken_at ? formatDate(photo.taken_at) : null)}</span
        >
      </div>
      <div class="meta-item">
        <label>{$t('ui.metadata.date_modified', { default: 'Date Modified:' })}</label><span
          id="meta-date-modified"
          style="opacity: {fieldOpacity(photo?.date_modified)}"
          >{setField(
            'meta-date-modified',
            photo?.date_modified ? formatDate(photo.date_modified) : null
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
          <label>{$t('ui.metadata.make', { default: 'Make:' })}</label><span
            id="meta-camera-make"
            style="opacity: {fieldOpacity(camera.make)}"
            >{setField('meta-camera-make', camera.make)}</span
          >
        </div>
        <div class="meta-item">
          <label>{$t('ui.metadata.model', { default: 'Model:' })}</label><span
            id="meta-camera-model"
            style="opacity: {fieldOpacity(camera.model)}"
            >{setField('meta-camera-model', camera.model)}</span
          >
        </div>
        <div class="meta-item">
          <label>{$t('ui.metadata.lens_make', { default: 'Lens Make:' })}</label><span
            id="meta-lens-make"
            style="opacity: {fieldOpacity(camera.lens_make)}"
            >{setField('meta-lens-make', camera.lens_make)}</span
          >
        </div>
        <div class="meta-item">
          <label>{$t('ui.metadata.lens_model', { default: 'Lens Model:' })}</label><span
            id="meta-lens-model"
            style="opacity: {fieldOpacity(camera.lens_model)}"
            >{setField('meta-lens-model', camera.lens_model)}</span
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
          <label>{$t('ui.metadata.iso', { default: 'ISO:' })}</label><span
            id="meta-iso"
            style="opacity: {fieldOpacity(settings.iso)}"
            >{setField('meta-iso', settings.iso ? `ISO ${settings.iso}` : null)}</span
          >
        </div>
        <div class="meta-item">
          <label>{$t('ui.metadata.aperture', { default: 'Aperture:' })}</label><span
            id="meta-aperture"
            style="opacity: {fieldOpacity(settings.aperture)}"
            >{setField(
              'meta-aperture',
              settings.aperture ? `f/${settings.aperture.toFixed(1)}` : null
            )}</span
          >
        </div>
        <div class="meta-item">
          <label>{$t('ui.metadata.shutter_speed', { default: 'Shutter Speed:' })}</label><span
            id="meta-shutter"
            style="opacity: {fieldOpacity(settings.shutter_speed)}"
            >{setField('meta-shutter', settings.shutter_speed)}</span
          >
        </div>
        <div class="meta-item">
          <label>{$t('ui.metadata.focal_length', { default: 'Focal Length:' })}</label><span
            id="meta-focal"
            style="opacity: {fieldOpacity(settings.focal_length)}"
            >{setField(
              'meta-focal',
              settings.focal_length ? `${settings.focal_length.toFixed(0)} mm` : null
            )}</span
          >
        </div>
        <div class="meta-item">
          <label>{$t('ui.metadata.exposure_mode', { default: 'Exposure Mode:' })}</label><span
            id="meta-exposure"
            style="opacity: {fieldOpacity(settings.exposure_mode)}"
            >{setField('meta-exposure', settings.exposure_mode)}</span
          >
        </div>
        <div class="meta-item">
          <label>{$t('ui.metadata.metering_mode', { default: 'Metering Mode:' })}</label><span
            id="meta-metering"
            style="opacity: {fieldOpacity(settings.metering_mode)}"
            >{setField('meta-metering', settings.metering_mode)}</span
          >
        </div>
        <div class="meta-item">
          <label>{$t('ui.metadata.white_balance', { default: 'White Balance:' })}</label><span
            id="meta-wb"
            style="opacity: {fieldOpacity(settings.white_balance)}"
            >{setField('meta-wb', settings.white_balance)}</span
          >
        </div>
        <div class="meta-item">
          <label>{$t('ui.metadata.flash', { default: 'Flash:' })}</label><span
            id="meta-flash"
            style="opacity: {fieldOpacity(settings.flash_used !== undefined)}"
            >{setField(
              'meta-flash',
              settings.flash_used !== undefined ? (settings.flash_used ? 'Yes' : 'No') : null
            )}</span
          >
        </div>
        <div class="meta-item">
          <label>{$t('ui.metadata.orientation', { default: 'Orientation:' })}</label><span
            id="meta-orientation"
            style="opacity: {fieldOpacity(photo?.orientation)}"
            >{setField('meta-orientation', photo?.orientation)}</span
          >
        </div>
        <div class="meta-item">
          <label>{$t('ui.metadata.color_space', { default: 'Color Space:' })}</label><span
            id="meta-colorspace"
            style="opacity: {fieldOpacity(settings.color_space)}"
            >{setField('meta-colorspace', settings.color_space)}</span
          >
        </div>
      </div>
    {/if}

    {#if hasLocation}
      <div class="meta-section" id="location-section">
        <h4 class="meta-section-title">
          {$t('ui.metadata.location_section', { default: 'Location' })}
        </h4>
        <div class="meta-item">
          <label>{$t('ui.metadata.gps', { default: 'GPS:' })}</label><span
            id="meta-gps"
            style="opacity: {fieldOpacity(location.latitude && location.longitude)}"
            >{setField(
              'meta-gps',
              location.latitude && location.longitude
                ? `${location.latitude.toFixed(6)}, ${location.longitude.toFixed(6)}`
                : null
            )}</span
          >
        </div>
        <div class="meta-item">
          <label>{$t('ui.metadata.location_name', { default: 'Location Name:' })}</label><span
            id="meta-location-name"
            style="opacity: 0.5">-</span
          >
        </div>
      </div>
    {/if}

    {#if isVideo}
      <div class="meta-section" id="video-section">
        <h4 class="meta-section-title">
          {$t('ui.metadata.video_section', { default: 'Video Information' })}
        </h4>
        <div class="meta-item">
          <label>{$t('ui.metadata.duration', { default: 'Duration:' })}</label><span
            id="meta-duration"
            style="opacity: {fieldOpacity(photo?.duration)}"
            >{setField(
              'meta-duration',
              photo?.duration ? formatDuration(photo.duration) : null
            )}</span
          >
        </div>
        <div class="meta-item">
          <label>{$t('ui.metadata.video_codec', { default: 'Video Codec:' })}</label><span
            id="meta-video-codec"
            style="opacity: {fieldOpacity(videoMeta.codec)}"
            >{setField('meta-video-codec', videoMeta.codec)}</span
          >
        </div>
        <div class="meta-item">
          <label>{$t('ui.metadata.audio_codec', { default: 'Audio Codec:' })}</label><span
            id="meta-audio-codec"
            style="opacity: {fieldOpacity(videoMeta.audio_codec)}"
            >{setField('meta-audio-codec', videoMeta.audio_codec)}</span
          >
        </div>
        <div class="meta-item">
          <label>{$t('ui.metadata.frame_rate', { default: 'Frame Rate:' })}</label><span
            id="meta-framerate"
            style="opacity: {fieldOpacity(videoMeta.frame_rate)}"
            >{setField(
              'meta-framerate',
              videoMeta.frame_rate ? `${videoMeta.frame_rate.toFixed(2)} fps` : null
            )}</span
          >
        </div>
        <div class="meta-item">
          <label>{$t('ui.metadata.bitrate', { default: 'Bitrate:' })}</label><span
            id="meta-bitrate"
            style="opacity: {fieldOpacity(videoMeta.bitrate)}"
            >{setField(
              'meta-bitrate',
              videoMeta.bitrate ? `${videoMeta.bitrate} kbps` : null
            )}</span
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
  }

  .photo-info-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
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
    color: var(--primary-color);
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

  .photo-meta .meta-item :global(label) {
    font-weight: var(--font-semibold);
    font-size: var(--font-base);
    color: var(--text-secondary);
  }

  .photo-meta .meta-item :global(span) {
    color: var(--text-primary);
    font-size: var(--font-base);
    font-weight: var(--font-medium);
  }

  .photo-meta-full .meta-item :global(label) {
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
    overflow-wrap: break-word;
    text-align: right;
    flex: 1;
  }

  .btn-icon {
    background: none;
    border: none;
    cursor: pointer;
    padding: 8px;
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
</style>
