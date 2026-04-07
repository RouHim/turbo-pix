class IndexingOrbitManager {
  constructor() {
    this.pollInterval = null;
    this.pollFrequencyIndexing = 1000;
    this.pollFrequencyIdle = 30000;
    this.hideDelay = 2000;
    this.hideTimeout = null;
    this.hasIndexedStorageKey = 'turbopix_has_indexed';
    this.ring = null;
    this.bottomSheet = null;
    this.svg = null;
    this.centerIcon = null;
    this.phaseElements = new Map();
    this.isIndexing = false;
    this.autoOpened = false;
  }

  init() {
    this.ring = document.querySelector('[data-phase-ring]');
    this.bottomSheet = document.querySelector('[data-bottom-sheet]');

    if (!this.ring) {
      console.warn('Indexing orbit ring element not found');
      return;
    }

    this.backdrop = document.createElement('div');
    this.backdrop.className = 'indexing-sheet-backdrop';
    document.body.appendChild(this.backdrop);

    this.ring.addEventListener('click', () => this.toggleSheet());
    this.backdrop.addEventListener('click', () => this.closeSheet());

    if (this.bottomSheet) {
      const closeBtn = this.bottomSheet.querySelector('[data-sheet-close]');
      if (closeBtn) {
        closeBtn.addEventListener('click', () => this.closeSheet());
      }
    }

    if (this.ring) {
      this.ring.setAttribute(
        'aria-label',
        this.getTranslation('ui.indexing_sheet_title', 'Indexing progress')
      );
    }

    if (this.bottomSheet) {
      const closeBtn = this.bottomSheet.querySelector('[data-sheet-close]');
      if (closeBtn) {
        closeBtn.setAttribute('aria-label', this.getTranslation('ui.close', 'Close'));
      }
    }

    document.addEventListener('keydown', (e) => {
      if (e.key === 'Escape') this.closeSheet();
    });

    this.renderOrbit();
    this.startPolling();
  }

  startPolling() {
    this.checkStatus();

    if (window.logger) {
      window.logger.debug('Indexing status polling started', {
        component: 'IndexingOrbit',
      });
    }
  }

  scheduleNextPoll() {
    if (this.pollInterval) {
      clearTimeout(this.pollInterval);
    }

    const frequency = this.isIndexing ? this.pollFrequencyIndexing : this.pollFrequencyIdle;

    this.pollInterval = setTimeout(() => {
      this.checkStatus();
    }, frequency);
  }

  stopPolling() {
    if (this.pollInterval) {
      clearTimeout(this.pollInterval);
      this.pollInterval = null;

      if (window.logger) {
        window.logger.debug('Indexing status polling stopped', {
          component: 'IndexingOrbit',
        });
      }
    }
  }

  async checkStatus() {
    try {
      const status = await window.api.getIndexingStatus();
      const normalizedStatus = this.normalizeStatus(status);
      const wasIndexing = this.isIndexing;

      if (normalizedStatus.is_indexing) {
        this.isIndexing = true;
        this.updateOrbit(normalizedStatus);
      } else if (wasIndexing) {
        this.markIndexingCompleted();
        this.isIndexing = false;
        this.hideRing({ showCompletionPulse: true });

        if (window.photoGrid) {
          window.photoGrid.loadPhotos();
        }
      } else {
        this.isIndexing = false;
        this.hideRing();
      }

      window.dispatchEvent(
        new CustomEvent('indexingStatusChanged', {
          detail: normalizedStatus,
        })
      );
    } catch (error) {
      if (window.logger) {
        window.logger.error('Failed to check indexing status', error, {
          component: 'IndexingOrbit',
        });
      } else {
        console.error('IndexingOrbit: Failed to check status', error);
      }
    } finally {
      this.scheduleNextPoll();
    }
  }

  normalizeStatus(status) {
    const phase = status.phase ?? status.active_phase_id ?? '';

    return {
      ...status,
      phase,
      active_phase_id: status.active_phase_id ?? phase,
      phases: Array.isArray(status.phases) ? status.phases : [],
    };
  }

  toggleSheet() {
    if (!this.ring || this.ring.getAttribute('data-ring-mode') !== 'compact') return;

    if (!this.bottomSheet) return;
    const isHidden = this.bottomSheet.getAttribute('aria-hidden') === 'true';

    if (isHidden) {
      this.openSheet();
    } else {
      this.closeSheet();
    }
  }

  openSheet(force = false) {
    if (!force && (!this.ring || this.ring.getAttribute('data-ring-mode') !== 'compact')) return;
    if (!this.bottomSheet) return;

    this.bottomSheet.setAttribute('aria-hidden', 'false');
    if (this.ring) {
      this.ring.setAttribute('aria-expanded', 'true');
    }
    if (this.backdrop) {
      this.backdrop.classList.add('is-visible');
    }
  }

  closeSheet() {
    if (!this.bottomSheet) return;

    this.bottomSheet.setAttribute('aria-hidden', 'true');
    if (this.ring) {
      this.ring.setAttribute('aria-expanded', 'false');
    }
    if (this.backdrop) {
      this.backdrop.classList.remove('is-visible');
    }
  }

  updateSheet(status) {
    if (!this.bottomSheet) return;

    const countEl = this.bottomSheet.querySelector('[data-sheet-photos-count]');
    if (countEl) {
      countEl.textContent = status.photos_indexed ?? 0;
    }

    let activePhaseName = '';
    let activePhaseProcessed = 0;
    let activePhaseTotal = 0;
    let currentItem = '';

    status.phases.forEach((phase) => {
      const row = this.bottomSheet.querySelector(`[data-phase-id="${phase.id}"]`);
      if (!row) return;

      const countSpan = row.querySelector('[data-phase-count]');
      const fillDiv = row.querySelector('[data-phase-fill]');
      const errorsSpan = row.querySelector('[data-phase-errors]');

      row.classList.remove('is-active', 'is-done', 'is-error');

      if (phase.state === 'active') {
        row.classList.add('is-active');
        activePhaseName = this.getTranslation(`ui.indexing_phase_${phase.id}`, phase.id);
        activePhaseProcessed = phase.processed || 0;
        activePhaseTotal = phase.total || 0;
        currentItem = phase.current_item || '';
      } else if (phase.state === 'done') {
        row.classList.add('is-done');
      } else if (phase.state === 'error') {
        row.classList.add('is-error');
      }

      if (phase.kind === 'determinate') {
        const total = phase.total || 0;
        const processed = phase.processed || 0;
        if (countSpan) {
          countSpan.textContent = `${processed}/${total}`;
        }
        if (fillDiv) {
          const percent = total > 0 ? Math.round((processed / total) * 100) : 0;
          fillDiv.style.width = `${percent}%`;
        }
      } else {
        if (countSpan) {
          countSpan.textContent = '—';
        }
        if (fillDiv) {
          fillDiv.style.width = '0%';
        }
      }

      if (errorsSpan) {
        if (phase.errors && phase.errors > 0) {
          const tpl = this.getTranslation('ui.indexing_sheet_errors', `${phase.errors} error(s)`);
          errorsSpan.textContent = tpl.replace('{{count}}', phase.errors);
        } else {
          errorsSpan.textContent = '';
        }
      }
    });

    const currentItemEl = this.bottomSheet.querySelector('[data-sheet-current-item]');
    if (currentItemEl) {
      currentItemEl.textContent = currentItem;
    }

    if (this.ring && this.ring.getAttribute('data-ring-mode') === 'compact' && activePhaseName) {
      const tpl = this.getTranslation(
        'ui.indexing_ring_tooltip',
        `${activePhaseName} — ${activePhaseProcessed}/${activePhaseTotal}`
      );
      this.ring.title = tpl
        .replace('{{phase}}', activePhaseName)
        .replace('{{processed}}', activePhaseProcessed)
        .replace('{{total}}', activePhaseTotal);
    } else if (this.ring) {
      this.ring.removeAttribute('title');
    }
  }

  renderOrbit() {
    this.ring.innerHTML = '';
    this.phaseElements.clear();

    const svgNamespace = 'http://www.w3.org/2000/svg';
    const wrapper = document.createElement('div');
    wrapper.className = 'indexing-orbit-shell';

    this.svg = document.createElementNS(svgNamespace, 'svg');
    this.svg.setAttribute('class', 'indexing-orbit-svg');
    this.svg.setAttribute('viewBox', '0 0 280 280');
    this.svg.setAttribute('aria-hidden', 'true');

    this.getPhases().forEach((phase, index) => {
      const segment = document.createElementNS(svgNamespace, 'path');
      segment.setAttribute('class', 'indexing-orbit-segment');
      segment.setAttribute('d', this.describeArc(140, 140, 120, index));
      segment.setAttribute('data-phase-id', phase.id);
      segment.setAttribute('data-phase-state', 'pending');
      segment.setAttribute('stroke', 'currentColor');
      segment.setAttribute('fill', 'none');
      this.svg.append(segment);
      this.phaseElements.set(phase.id, segment);
    });

    this.centerIcon = document.createElement('div');
    this.centerIcon.className = 'orbit-center-icon';
    this.centerIcon.innerHTML = '<i data-feather="camera"></i>';

    wrapper.append(this.svg, this.centerIcon);
    this.ring.append(wrapper);

    if (typeof feather !== 'undefined') {
      feather.replace();
    }
  }

  updateOrbit(status) {
    if (!this.ring) {
      return;
    }

    this.cancelPendingHide();

    const mode = this.determineMode(status);
    this.ring.setAttribute('data-ring-mode', mode);
    if (mode === 'large' && !this.autoOpened) {
      this.autoOpened = true;
      window.requestAnimationFrame(() => this.openSheet(true));
    }
    if (!status.is_indexing) {
      this.updateCenterIcon('discovering');
      return;
    }

    this.ring.setAttribute(
      'aria-label',
      this.getTranslation('ui.indexing_photos', 'Processing your photos...')
    );

    this.resetSegments(false);

    const arcLength = 125.66;

    status.phases.forEach((phase) => {
      const segment = this.phaseElements.get(phase.id);
      if (!segment) {
        return;
      }

      segment.setAttribute('data-phase-state', phase.state || 'pending');

      if (phase.state === 'done') {
        segment.style.strokeDashoffset = '0';
      } else if (phase.state === 'active' && phase.kind === 'determinate' && phase.total > 0) {
        const progress = Math.min(Math.max(phase.processed / phase.total, 0), 1);
        segment.style.strokeDashoffset = (arcLength * (1 - progress)).toString();
      } else {
        segment.style.strokeDashoffset = arcLength.toString();
      }

      this.updateOrbitDot(phase);
    });

    this.updateCenterIcon(status.phase);
    this.updateSheet(status);
  }

  determineMode(status) {
    if (!status.is_indexing) {
      return 'hidden';
    }

    if (this.hasIndexedBefore()) {
      return 'compact';
    }

    const photosIndexed = Number(status.photos_indexed ?? 0);

    if (photosIndexed === 0) {
      return 'large';
    }

    return 'compact';
  }

  updateCenterIcon(activePhaseId) {
    if (!this.centerIcon) {
      return;
    }

    const iconName = this.getPhaseIcon(activePhaseId);
    this.centerIcon.innerHTML = `<i data-feather="${iconName}"></i>`;

    if (typeof feather !== 'undefined') {
      feather.replace();
    }
  }

  resetSegments(resetAll = true) {
    const arcLength = 125.66;
    this.phaseElements.forEach((segment, phaseId) => {
      if (resetAll) {
        segment.setAttribute('data-phase-state', 'pending');
        segment.style.strokeDashoffset = arcLength.toString();
      }

      this.removeOrbitDot(phaseId);
    });
  }

  markSegmentsDone() {
    this.phaseElements.forEach((segment, phaseId) => {
      segment.setAttribute('data-phase-state', 'done');
      segment.style.strokeDashoffset = '0';
      this.removeOrbitDot(phaseId);
    });
  }

  updateOrbitDot(phase) {
    const svgNamespace = 'http://www.w3.org/2000/svg';
    let group = this.svg.querySelector(`g[data-orbit-phase="${phase.id}"]`);

    if (phase.state === 'active' && phase.kind === 'indeterminate') {
      if (!group) {
        group = document.createElementNS(svgNamespace, 'g');
        group.setAttribute('data-orbit-phase', phase.id);
        group.style.transformOrigin = '140px 140px';
        group.style.transformBox = 'fill-box';
        group.style.animation = this.prefersReducedMotion()
          ? 'none'
          : 'orbit-segment 2s ease-in-out infinite';

        const circle = document.createElementNS(svgNamespace, 'circle');
        const index = this.getPhases().findIndex((p) => p.id === phase.id);
        const midpointAngle = -90 + index * 60 + 30;
        const pos = this.polarToCartesian(140, 140, 120, midpointAngle);

        circle.setAttribute('cx', pos.x);
        circle.setAttribute('cy', pos.y);
        circle.setAttribute('class', 'orbit-dot');
        circle.setAttribute('data-orbit-dot', 'true');

        group.appendChild(circle);
        this.svg.appendChild(group);
      }
    } else if (group) {
      group.remove();
    }
  }

  removeOrbitDot(phaseId) {
    const group = this.svg.querySelector(`g[data-orbit-phase="${phaseId}"]`);
    if (group) {
      group.remove();
    }
  }

  hideRing({ showCompletionPulse = false } = {}) {
    if (!this.ring) {
      return;
    }

    this.cancelPendingHide();

    if (showCompletionPulse) {
      this.markSegmentsDone();
      this.updateCenterIcon('housekeeping');
      this.hideTimeout = setTimeout(() => {
        this.closeSheet();
        this.applyHiddenState();
      }, this.hideDelay);
      return;
    }

    this.closeSheet();
    this.applyHiddenState();
  }

  applyHiddenState() {
    if (!this.ring) {
      return;
    }

    this.ring.setAttribute('data-ring-mode', this.determineMode({ is_indexing: false }));
    this.updateCenterIcon('discovering');
    this.resetSegments();
  }

  cancelPendingHide() {
    if (!this.hideTimeout) {
      return;
    }

    clearTimeout(this.hideTimeout);
    this.hideTimeout = null;
  }

  hasIndexedBefore() {
    try {
      return window.localStorage.getItem(this.hasIndexedStorageKey) === 'true';
    } catch {
      return false;
    }
  }

  prefersReducedMotion() {
    return window.matchMedia?.('(prefers-reduced-motion: reduce)')?.matches ?? false;
  }

  markIndexingCompleted() {
    try {
      window.localStorage.setItem(this.hasIndexedStorageKey, 'true');
    } catch {
      return;
    }
  }

  getPhases() {
    return [
      { id: 'discovering', icon: 'camera' },
      { id: 'metadata', icon: 'file-text' },
      { id: 'semantic_vectors', icon: 'cpu' },
      { id: 'geo_resolution', icon: 'map-pin' },
      { id: 'collages', icon: 'grid' },
      { id: 'housekeeping', icon: 'check-circle' },
    ];
  }

  getPhaseIcon(phaseId) {
    const phase = this.getPhases().find((entry) => entry.id === phaseId);
    return phase?.icon || 'camera';
  }

  getTranslation(key, fallback) {
    return window.i18nManager?.t?.(key) || fallback;
  }

  describeArc(centerX, centerY, radius, index) {
    const gapDegrees = 4;
    const segmentDegrees = 60;
    const startAngle = -90 + index * segmentDegrees + gapDegrees / 2;
    const endAngle = startAngle + segmentDegrees - gapDegrees;
    const start = this.polarToCartesian(centerX, centerY, radius, startAngle);
    const end = this.polarToCartesian(centerX, centerY, radius, endAngle);

    return ['M', start.x, start.y, 'A', radius, radius, 0, 0, 1, end.x, end.y].join(' ');
  }

  polarToCartesian(centerX, centerY, radius, angleInDegrees) {
    const angleInRadians = ((angleInDegrees - 90) * Math.PI) / 180;

    return {
      x: centerX + radius * Math.cos(angleInRadians),
      y: centerY + radius * Math.sin(angleInRadians),
    };
  }

  destroy() {
    this.cancelPendingHide();
    this.stopPolling();
  }
}

window.indexingStatus = new IndexingOrbitManager();
