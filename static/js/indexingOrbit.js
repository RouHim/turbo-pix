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

    this.resetSegments(false);

    const arcLength = 125.66;

    status.phases.forEach((phase) => {
      const segment = this.phaseElements.get(phase.id);
      if (!segment) {
        return;
      }

      segment.setAttribute('data-phase-state', phase.state || 'pending');

      // Update dash offset
      if (phase.state === 'done') {
        segment.style.strokeDashoffset = '0';
      } else if (phase.state === 'active' && phase.kind === 'determinate' && phase.total > 0) {
        const progress = Math.min(Math.max(phase.processed / phase.total, 0), 1);
        segment.style.strokeDashoffset = (arcLength * (1 - progress)).toString();
      } else {
        segment.style.strokeDashoffset = arcLength.toString();
      }

      // Update orbit dot
      this.updateOrbitDot(phase);
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

  resetSegments(resetAll = true) {
    const arcLength = 125.66;
    this.phaseElements.forEach((segment, phaseId) => {
      if (resetAll) {
        segment.setAttribute('data-phase-state', 'pending');
        segment.style.strokeDashoffset = arcLength.toString();
        this.removeOrbitDot(phaseId);
      }
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
        group.style.animation = 'orbit-segment 2s ease-in-out infinite';

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
