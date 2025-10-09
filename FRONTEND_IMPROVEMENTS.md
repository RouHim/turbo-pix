# Frontend Improvements Roadmap

> Generated: 2025-10-09
> Status: Planning Phase

## Overview

This document outlines recommended improvements for the TurboPix frontend codebase. The project currently uses vanilla JavaScript with ~6,000 lines of code across 12 JS files, 3 CSS files, and 1 HTML file.

---

### Template Security (XSS Prevention)
**Current State:** Multiple uses of `innerHTML` with string concatenation
**Files Affected:** `photoGrid.js:233`, `search.js:251`, `viewer.js`

**Vulnerabilities:**
```javascript
// UNSAFE - in photoGrid.js
card.innerHTML = `
  <div class="photo-card-title">${this.getPhotoTitle(photo)}</div>
`;
```

**Recommendation:**
- Use DOM APIs (`createElement`, `textContent`)
- Or sanitize with DOMPurify
- Or use a template engine (lit-html, handlebars)

---

### Linting & Formatting Setup
**Current State:** ESLint configured but enforcement unclear

**Actions:**
- [ ] Ensure ESLint runs on all files
- [ ] Add Prettier for consistent formatting

---

### Component Refactoring
**Current State:** Large files with multiple responsibilities

**Files to Break Down:**
- `viewer.js` (787 lines) → ViewerComponent, ViewerControls, ViewerMetadata
- `photoGrid.js` (587 lines) → PhotoGrid, PhotoCard, InfiniteScroll
- `app.js` (578 lines) → App, Navigation, ThemeManager

### Virtual Scrolling
**Current State:** All photos rendered in DOM
**Issue:** Performance degrades with 1000+ photos

**Recommendation:** Implement virtual scrolling

**Libraries:**
- `tanstack-virtual` (framework agnostic)
- Custom implementation with IntersectionObserver

### Image Optimization
**Improvements:**
- Use native `loading="lazy"` attribute
- Progressive image loading (blur → full)
- WebP format with fallbacks
- Responsive images with `srcset`

---

### CSS Architecture
**Current State:** 1,945 lines across 3 CSS files
**Recommendations:**
- [ ] Extract utility classes
- [ ] Consider CSS modules or styled-components
- [ ] Use CSS custom properties more extensively
- [ ] Reduce specificity complexity

## 📝 Notes

- Breaking changes are acceptable (per CLAUDE.md)
- Database can be recreated if needed
- Follow KISS + SOLID principles
- Maintain zero warnings policy
- Prefer iterator chains over loops (Rust style)
- Prefer no additional dependencies unless justified
- Prioritize readability and maintainability
