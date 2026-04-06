class IndexingOrbitManager {
  constructor() {
    this.pollInterval = null;
    this.pollFrequencyIndexing = 1000;
    this.pollFrequencyIdle = 30000;
    this.ring = null;
    this.bottomSheet = null;
    this.svg = null;
    this.centerIcon = null;
    this.phaseElements = new Map();
    this.isIndexing = false;
  }

  init() {
    this.ring = document.querySelector('[data-phase-ring]');
    this.bottomSheet = document.querySelector('[data-bottom-sheet]');

    if (!this.ring) {
      console.warn('Indexing orbit ring element not found');
      return;
    }

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

      this.updateOrbit(normalizedStatus);
      window.dispatchEvent(
        new CustomEvent('indexingStatusChanged', {
          detail: normalizedStatus,
        })
      );

      if (normalizedStatus.is_indexing) {
        this.isIndexing = true;
      } else if (this.isIndexing) {
        this.hideRing();
        this.isIndexing = false;

        if (window.photoGrid) {
          window.photoGrid.loadPhotos();
        }
      } else {
        this.hideRing();
      }
    } catch (error) {
      if (window.logger) {
        window.logger.error('Failed to check indexing status', error, {
          component: 'IndexingOrbit',
        });
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

    if (!status.is_indexing) {
      this.resetSegments();
      return;
    }

    this.ring.setAttribute('data-ring-mode', 'large');
    this.ring.setAttribute(
      'aria-label',
      this.getTranslation('ui.indexing_photos', 'Processing your photos...')
    );

    this.resetSegments();
    status.phases.forEach((phase) => {
      const segment = this.phaseElements.get(phase.id);
      if (!segment) {
        return;
      }

      segment.setAttribute('data-phase-state', phase.state || 'pending');
    });

    this.updateCenterIcon(status.phase);
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

  resetSegments() {
    this.phaseElements.forEach((segment) => {
      segment.setAttribute('data-phase-state', 'pending');
    });
  }

  hideRing() {
    if (!this.ring) {
      return;
    }

    this.resetSegments();
    this.ring.setAttribute('data-ring-mode', 'hidden');
    this.updateCenterIcon('discovering');
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
    const start = this.polarToCartesian(centerX, centerY, radius, endAngle);
    const end = this.polarToCartesian(centerX, centerY, radius, startAngle);

    return ['M', start.x, start.y, 'A', radius, radius, 0, 0, 0, end.x, end.y].join(' ');
  }

  polarToCartesian(centerX, centerY, radius, angleInDegrees) {
    const angleInRadians = ((angleInDegrees - 90) * Math.PI) / 180;

    return {
      x: centerX + radius * Math.cos(angleInRadians),
      y: centerY + radius * Math.sin(angleInRadians),
    };
  }

  destroy() {
    this.stopPolling();
  }
}

window.indexingStatus = new IndexingOrbitManager();
