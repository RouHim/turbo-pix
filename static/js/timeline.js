// Timeline Slider Component
class TimelineSlider {
  constructor() {
    this.data = null;
    this.currentFilter = null;
    this.debounceTimer = null;
    this.hoveredIndex = null;
    this.selectedIndex = null;

    // DOM elements
    this.container = document.querySelector('.timeline-container');
    this.slider = document.querySelector('.timeline-input');
    this.label = document.querySelector('.timeline-label');
    this.canvas = document.querySelector('.timeline-heatmap');
    this.resetBtn = document.querySelectorAll('.timeline-reset');
    this.yearSelect = document.querySelector('.timeline-year-select');
    this.monthSelect = document.querySelector('.timeline-month-select');

    // Create tooltip element
    this.tooltip = this.createTooltip();

    this.init();
  }

  async init() {
    try {
      await this.fetchTimelineData();
      this.setupEventListeners();
      this.renderHeatmap();
      this.populateDropdowns();
    } catch (error) {
      console.error('Failed to initialize timeline:', error);
      this.container.style.display = 'none';
    }
  }

  async fetchTimelineData() {
    const response = await fetch('/api/photos/timeline');
    if (!response.ok) throw new Error('Failed to fetch timeline data');
    this.data = await response.json();

    // Build positions array for slider mapping
    if (this.data.density && this.data.density.length > 0) {
      this.positions = this.data.density.map((d) => ({
        year: d.year,
        month: d.month,
        count: d.count,
      }));
    } else {
      this.positions = [];
    }
  }

  setupEventListeners() {
    // Slider events
    if (this.slider) {
      this.slider.addEventListener('input', (e) => this.handleSliderInput(e));
      this.slider.addEventListener('dblclick', () => this.resetFilter());

      // Update slider max to match number of positions
      if (this.positions.length > 0) {
        this.slider.max = this.positions.length - 1;
        this.slider.value = this.positions.length - 1; // Start at most recent
      }
    }

    // Canvas interaction events
    if (this.canvas) {
      this.canvas.addEventListener('click', (e) => this.handleCanvasClick(e));
      this.canvas.addEventListener('mousemove', (e) => this.handleCanvasHover(e));
      this.canvas.addEventListener('mouseleave', () => this.handleCanvasLeave());
      this.canvas.style.cursor = 'pointer';
    }

    // Reset button
    this.resetBtn.forEach((btn) => {
      btn.addEventListener('click', () => this.resetFilter());
    });

    // Mobile dropdowns
    if (this.yearSelect) {
      this.yearSelect.addEventListener('change', () => this.handleDropdownChange());
    }
    if (this.monthSelect) {
      this.monthSelect.addEventListener('change', () => this.handleDropdownChange());
    }
  }

  handleSliderInput(e) {
    const index = parseInt(e.target.value);

    if (index >= this.positions.length - 1) {
      // At the end = show all
      const allDatesText = window.i18nManager ? window.i18nManager.t('ui.all_dates') : 'All Dates';
      this.updateLabel(allDatesText);
      this.currentFilter = null;
      this.selectedIndex = null;
    } else {
      const pos = this.positions[index];
      this.updateLabel(this.formatDate(pos.year, pos.month));
      this.currentFilter = { year: pos.year, month: pos.month };
      this.selectedIndex = index;
    }

    // Re-render to show selection
    this.renderHeatmap();

    // Debounced filter update
    clearTimeout(this.debounceTimer);
    this.debounceTimer = setTimeout(() => {
      this.applyFilter();
    }, 300);
  }

  handleDropdownChange() {
    const year = this.yearSelect.value;
    const month = this.monthSelect.value;

    if (!year && !month) {
      this.currentFilter = null;
    } else {
      this.currentFilter = {
        year: year ? parseInt(year) : null,
        month: month ? parseInt(month) : null,
      };
    }

    this.applyFilter();
  }

  applyFilter() {
    if (window.turboPixApp && typeof window.turboPixApp.applyTimelineFilter === 'function') {
      window.turboPixApp.applyTimelineFilter(this.currentFilter);
    }
  }

  resetFilter() {
    this.currentFilter = null;
    this.selectedIndex = null;

    // Reset slider
    if (this.slider && this.positions.length > 0) {
      this.slider.value = this.positions.length - 1;
    }

    // Reset dropdowns
    if (this.yearSelect) this.yearSelect.value = '';
    if (this.monthSelect) this.monthSelect.value = '';

    const allDatesText = window.i18nManager ? window.i18nManager.t('ui.all_dates') : 'All Dates';
    this.updateLabel(allDatesText);

    // Re-render to clear selection
    this.renderHeatmap();

    this.applyFilter();
  }

  updateLabel(text) {
    if (this.label) {
      this.label.textContent = text;
    }
  }

  formatDate(year, month) {
    const monthKeys = [
      'january',
      'february',
      'march',
      'april',
      'may',
      'june',
      'july',
      'august',
      'september',
      'october',
      'november',
      'december',
    ];
    const monthKey = monthKeys[month - 1];
    const monthName = window.i18nManager
      ? window.i18nManager.t(`ui.months.${monthKey}`)
      : monthKey.charAt(0).toUpperCase() + monthKey.slice(1);
    return `${monthName} ${year}`;
  }

  createTooltip() {
    const tooltip = document.createElement('div');
    tooltip.className = 'timeline-tooltip';
    tooltip.style.display = 'none';
    document.body.appendChild(tooltip);
    return tooltip;
  }

  getBarIndexFromX(x) {
    if (!this.canvas || this.positions.length === 0) return null;

    const rect = this.canvas.getBoundingClientRect();
    const canvasX = x - rect.left;
    const barWidth = this.canvas.width / this.positions.length;
    const index = Math.floor(canvasX / barWidth);

    return index >= 0 && index < this.positions.length ? index : null;
  }

  handleCanvasClick(e) {
    const index = this.getBarIndexFromX(e.clientX);
    if (index === null) return;

    const pos = this.positions[index];
    this.currentFilter = { year: pos.year, month: pos.month };
    this.selectedIndex = index;

    // Update slider position
    if (this.slider) {
      this.slider.value = index;
    }

    // Update label
    this.updateLabel(this.formatDate(pos.year, pos.month));

    // Re-render heatmap with selection highlight
    this.renderHeatmap();

    // Apply filter immediately (no debounce for clicks)
    this.applyFilter();
  }

  handleCanvasHover(e) {
    const index = this.getBarIndexFromX(e.clientX);

    if (index !== this.hoveredIndex) {
      this.hoveredIndex = index;

      if (index !== null) {
        const pos = this.positions[index];
        const dateStr = this.formatDate(pos.year, pos.month);
        const photosText = window.i18nManager
          ? window.i18nManager.t('ui.photos_count', { count: pos.count })
          : `${pos.count} photos`;

        this.tooltip.innerHTML = `<div class="timeline-tooltip-date">${dateStr}</div><div class="timeline-tooltip-count">${photosText}</div>`;
        this.tooltip.style.display = 'block';
        this.tooltip.style.left = `${e.clientX}px`;
        this.tooltip.style.top = `${e.clientY - 60}px`;
      } else {
        this.tooltip.style.display = 'none';
      }

      // Re-render with hover effect
      this.renderHeatmap();
    } else if (index !== null) {
      // Update tooltip position
      this.tooltip.style.left = `${e.clientX}px`;
      this.tooltip.style.top = `${e.clientY - 60}px`;
    }
  }

  handleCanvasLeave() {
    this.hoveredIndex = null;
    this.tooltip.style.display = 'none';
    this.renderHeatmap();
  }

  renderHeatmap() {
    if (!this.canvas || !this.data || !this.data.density) return;

    const ctx = this.canvas.getContext('2d');
    const width = this.canvas.width;
    const height = this.canvas.height;

    // Clear canvas
    ctx.clearRect(0, 0, width, height);

    if (this.positions.length === 0) return;

    // Find max count for normalization
    const maxCount = Math.max(...this.data.density.map((d) => d.count));

    // Draw year markers and labels
    this.drawYearMarkers(ctx, width, height);

    // Draw bars for each month
    const barWidth = width / this.positions.length;
    this.positions.forEach((pos, index) => {
      const normalizedHeight = (pos.count / maxCount) * height;
      const x = index * barWidth;
      const y = height - normalizedHeight;

      // Determine bar state
      const isHovered = this.hoveredIndex === index;
      const isSelected = this.selectedIndex === index;

      // Base opacity based on density
      let opacity = 0.3 + (pos.count / maxCount) * 0.7;

      // Hover effect: increase opacity and scale
      if (isHovered && !isSelected) {
        opacity = Math.min(opacity + 0.2, 1);
      }

      ctx.fillStyle = `rgba(99, 102, 241, ${opacity})`;
      ctx.fillRect(x, y, barWidth - 1, normalizedHeight);

      // Draw selection highlight
      if (isSelected) {
        ctx.strokeStyle = 'rgba(99, 102, 241, 1)';
        ctx.lineWidth = 3;
        ctx.strokeRect(x + 1, y, barWidth - 3, normalizedHeight);

        // Add glow effect
        ctx.shadowBlur = 8;
        ctx.shadowColor = 'rgba(99, 102, 241, 0.6)';
        ctx.fillStyle = 'rgba(99, 102, 241, 0.9)';
        ctx.fillRect(x, y, barWidth - 1, normalizedHeight);
        ctx.shadowBlur = 0;
      }

      // Draw hover highlight
      if (isHovered && !isSelected) {
        ctx.fillStyle = 'rgba(255, 255, 255, 0.1)';
        ctx.fillRect(x, y, barWidth - 1, normalizedHeight);
      }
    });
  }

  drawYearMarkers(ctx, width, height) {
    if (this.positions.length === 0) return;

    const barWidth = width / this.positions.length;
    let lastYear = null;

    this.positions.forEach((pos, index) => {
      if (pos.year !== lastYear) {
        const x = index * barWidth;

        // Draw year divider line (subtle)
        if (lastYear !== null) {
          ctx.strokeStyle = 'rgba(99, 102, 241, 0.15)';
          ctx.lineWidth = 1;
          ctx.beginPath();
          ctx.moveTo(x, 0);
          ctx.lineTo(x, height);
          ctx.stroke();
        }

        lastYear = pos.year;
      }
    });
  }

  populateDropdowns() {
    if (!this.data || !this.data.density) return;

    // Get unique years
    const years = [...new Set(this.data.density.map((d) => d.year))].sort((a, b) => b - a);

    // Populate year select
    if (this.yearSelect) {
      years.forEach((year) => {
        const option = document.createElement('option');
        option.value = year;
        option.textContent = year;
        this.yearSelect.appendChild(option);
      });
    }

    // Populate month select
    if (this.monthSelect) {
      const monthKeys = [
        'january',
        'february',
        'march',
        'april',
        'may',
        'june',
        'july',
        'august',
        'september',
        'october',
        'november',
        'december',
      ];
      monthKeys.forEach((key, index) => {
        const option = document.createElement('option');
        option.value = index + 1;
        option.textContent = window.i18nManager
          ? window.i18nManager.t(`ui.months.${key}`)
          : key.charAt(0).toUpperCase() + key.slice(1);
        this.monthSelect.appendChild(option);
      });
    }
  }
}

// Initialize timeline when DOM is ready
if (document.readyState === 'loading') {
  document.addEventListener('DOMContentLoaded', () => {
    window.timelineSlider = new TimelineSlider();
  });
} else {
  window.timelineSlider = new TimelineSlider();
}
