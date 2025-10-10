class InfiniteScroll {
  constructor(grid, options = {}) {
    this.grid = grid;
    this.options = {
      threshold: options.threshold || 800,
      throttleDelay: options.throttleDelay || 250,
    };

    this.scrollContainer = utils.$('.main-content');
    this.loadMoreContainer = utils.$('#load-more-container');

    this.init();
  }

  init() {
    if (this.scrollContainer) {
      utils.on(
        this.scrollContainer,
        'scroll',
        utils.throttle(() => {
          this.checkScrollPosition();
        }, this.options.throttleDelay)
      );
    }

    this.setupLoadMoreButton();
  }

  setupLoadMoreButton() {
    const loadMoreBtn = utils.$('#load-more-btn');
    if (loadMoreBtn) {
      utils.on(loadMoreBtn, 'click', () => this.grid.loadMore());
    }
  }

  checkScrollPosition() {
    if (this.grid.loading || !this.grid.hasMore) return;

    if (!this.scrollContainer) return;

    const scrollTop = this.scrollContainer.scrollTop;
    const containerHeight = this.scrollContainer.clientHeight;
    const scrollHeight = this.scrollContainer.scrollHeight;
    const distanceFromBottom = scrollHeight - (scrollTop + containerHeight);

    if (window.logger) {
      window.logger.debug('Scroll position check', {
        scrollTop,
        containerHeight,
        scrollHeight,
        distanceFromBottom,
        loading: Boolean(this.grid.loading),
        hasMore: Boolean(this.grid.hasMore),
      });
    }

    if (distanceFromBottom <= this.options.threshold) {
      if (window.logger) {
        window.logger.info('Infinite scroll triggered', {
          distanceFromBottom,
        });
      }
      this.grid.loadMore();
    }
  }

  updateLoadingIndicator() {
    if (!this.loadMoreContainer) return;

    if (this.grid.loading && this.grid.photos.length > 0) {
      this.loadMoreContainer.style.display = 'flex';
      this.loadMoreContainer.innerHTML = `
        <div class="infinite-scroll-loading">
          <div class="dot-wave">
            <div class="dot-wave-dot"></div>
            <div class="dot-wave-dot"></div>
            <div class="dot-wave-dot"></div>
          </div>
        </div>
      `;
    } else if (!this.grid.loading && !this.grid.hasMore && this.grid.photos.length > 0) {
      this.loadMoreContainer.style.display = 'flex';
      this.loadMoreContainer.innerHTML = `
        <div class="infinite-scroll-end">
          <div class="end-dots">
            <div class="end-dot"></div>
            <div class="end-dot"></div>
            <div class="end-dot"></div>
          </div>
        </div>
      `;
    } else {
      this.loadMoreContainer.style.display = 'none';
    }
  }

  recheckAfterLoad() {
    window.requestAnimationFrame(() => {
      setTimeout(() => {
        this.checkScrollPosition();
      }, 50);
    });
  }
}

window.InfiniteScroll = InfiniteScroll;
