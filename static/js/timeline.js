// Timeline Slider Component
class TimelineSlider {
  constructor() {
    this.data = null;
    this.currentFilter = null;
    this.debounceTimer = null;

    // DOM elements
    this.container = document.querySelector('.timeline-container');
    this.slider = document.querySelector('.timeline-input');
    this.label = document.querySelector('.timeline-label');
    this.canvas = document.querySelector('.timeline-heatmap');
    this.resetBtn = document.querySelectorAll('.timeline-reset');
    this.yearSelect = document.querySelector('.timeline-year-select');
    this.monthSelect = document.querySelector('.timeline-month-select');

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
    } else {
      const pos = this.positions[index];
      this.updateLabel(this.formatDate(pos.year, pos.month));
      this.currentFilter = { year: pos.year, month: pos.month };
    }

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

    // Reset slider
    if (this.slider && this.positions.length > 0) {
      this.slider.value = this.positions.length - 1;
    }

    // Reset dropdowns
    if (this.yearSelect) this.yearSelect.value = '';
    if (this.monthSelect) this.monthSelect.value = '';

    const allDatesText = window.i18nManager ? window.i18nManager.t('ui.all_dates') : 'All Dates';
    this.updateLabel(allDatesText);
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

    // Draw bars for each month
    const barWidth = width / this.positions.length;
    this.positions.forEach((pos, index) => {
      const normalizedHeight = (pos.count / maxCount) * height;
      const x = index * barWidth;
      const y = height - normalizedHeight;

      // Gradient from light to primary color based on density
      const opacity = 0.3 + (pos.count / maxCount) * 0.7;
      ctx.fillStyle = `rgba(99, 102, 241, ${opacity})`; // primary-color with opacity

      ctx.fillRect(x, y, barWidth - 1, normalizedHeight);
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
