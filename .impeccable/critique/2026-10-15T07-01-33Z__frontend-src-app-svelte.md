---
target: frontend/src/App.svelte (Hauptansicht)
total_score: 21
max_score: 40
na_heuristics: 
p0_count: 2
p1_count: 3
target_identity: "file:/home/rouven/projects/turbo-pix/frontend/src/App.svelte"
target_fingerprint: "sha256:d6841ee05b98aa1f74335cdc3e35e1de0f34efa379d6f9640ade2e477e626bf4"
target_path: /home/rouven/projects/turbo-pix/frontend/src/App.svelte
timestamp: 2026-10-15T07-01-33Z
slug: frontend-src-app-svelte
closed: true
---
# Kritik: TurboPix — Hauptansicht (Gallery View)

Method: dual-agent (A: CritiqueA-2 · B: CritiqueB)
Target: `frontend/src/App.svelte` (Haupt-Galerieansicht) · Live: http://localhost:5173 (headless Chromium, ~17 Testmedien; geprüft hell+dunkel, 1440×900 + 390×844)

## Design Health Score

| # | Heuristik | Score | Kernbefund |
|---|-----------|-------|------------|
| 1 | Sichtbarkeit des Systemstatus | 2 | Skeleton bei 1,117:1 praktisch unsichtbar; `#photo-grid` ohne `role`/`aria-busy`; keine Trefferzahl („17 photos" auch bei 10 Suchtreffern) |
| 2 | System ↔ reale Welt | 2 | „All media" / „17 photos" / „No Photos Found" — drei Namen für dasselbe; Hint lehrt `location:city`, das als Literal nie trifft |
| 3 | Nutzerkontrolle & Freiheit | 3 | Escape leert Suche, ×-Reset der Timeline, URL-Back, „batch-exit"; aber kein Clear-× im Suchfeld, Leerzustand ohne Ausweg |
| 4 | Konsistenz & Standards | 2 | Unstylete native Selects; Glas ohne Solid-Fallback (Hint/Suggestions/SelectionBar); `window.confirm()` statt Dialog-Flavor; undefinierte Tokens (`--color-bg`, `--shadow-lg`); `:focus-visible` fehlt an 6 Controls |
| 5 | Fehlervermeidung | 2 | Batch-Delete visuell wie „Add to Favorites"; semantische Suche ohne Relevanzschwelle/Nullzustand |
| 6 | Wiedererkennen statt Erinnern | 2 | Unbeschriftete Icon-Kreise; mobil Titel+Meta gelöscht; Filterkontext scrollt unter den fixen Header |
| 7 | Flexibilität & Effizienz | 3 | Live-Suche 300 ms, URL-State, Saved Searches, Shift-Range, Granularitäts-Pills; aber ~21 Tab-Stops bis zur ersten Karte |
| 8 | Ästhetik & Minimalismus | 2 | Timeline-Karte 210 px (43 % des Viewports); erste Karte bei y=384; ~380 px tote Header-Mitte |
| 9 | Fehlererkennung & -behebung | 2 | Leerer Suchzustand ohne Aktion; Timeline-Status bleibt nach Leer-Klick als Dauerzustand stehen |
| 10 | Hilfe & Dokumentation | 1 | Nur der 13-px-Hint (3,02:1 dunkel / 3,67:1 hell), nicht per `aria-describedby` verknüpft, überlagert von den Suggestions |
| **Summe** | | **21/40** | **Acceptable** (52,5 %) |

## Design-Specificity-Verdikt

**LLM (Assessment A):** Die Chrome ist kategorie-austauschbar, der Inhalt nicht — und der einzige gerahmte Gegenstand auf dem Bildschirm ist ausgerechnet der Filter. Header (64 px Glas) und Sidebar (280 px, 7 Textbuttons) sind Standard-Admin-Shell; kein Element sagt „Fotobibliothek". Identität entsteht aus Playfair Display + warmem Papier + Slate/Brass — Stilhaltung, nicht Struktur. Produktspezifisch sind nur das Kontaktbogen-Mosaik (mobil `repeat(3,1fr)`, 2-px-Fugen, Radius 0) und die Timeline-Leiste — und beide werden entwertet: Foto-Karte gegen Canvas = **1,045:1** Kontrast, während die Timeline-Karte als einziger Block Border+Radius+Schatten trägt. Hierarchie invertiert: der Filter ist der Held der Seite.

**Deterministischer Scan (Assessment B):** `impeccable detect` = 11 Findings (Exit 2); Fundkonzentration `frontend/src/app.css` (10/11); App.svelte isoliert = 0. Severity: 1 warning (`border-accent-on-rounded`, Spinner `border-top: 4px solid` auf `radius-full`, app.css:947), 10 advisories (4× `design-system-font-size`, 3× `design-system-color` = `#ef4444`/`rgb(239 68 68 / …)` app.css:1363–1366, 2× `bounce-easing`, 1× `design-system-radius` 2px Drag-Handle). Header/Sidebar/SearchBar/PhotoGrid/TimelineSlider je 0.

**In-Page-Injection (B):** Mutation-Preflight ok; `detect.js` injiziert → Console `[impeccable] 3 anti-patterns found`: (1) Suchfeld-Placeholder **4,3:1** (< 4,5:1), (2)+(3) Spring-Easing am `IndexingOrbit` (Ring + Center-Icon). 3× `.impeccable-overlay` im DOM.
→ **Kein user-sichtbares Overlay:** der managed Browser dieser Maschine ist headless-only (`headed: true` schlägt fehl). Fallback-Signal: Console-Report + DOM-Zählung.

**Abgleich:** Übereinstimmung bei hartkodierten Farben und toten Tokens (A+B), bei Kontrastthemen (A rechnet 3,02/3,67/2,18:1; B misst live 4,3:1). Zusätzlich vom Detektor gefunden: Spinner-Border-Warning, 22-px-Download-Icon, 2-px-Drag-Handle, 2× `bounce-easing`. Falsch-positiv/nicht zwingend: 48 px und 28 px stehen in der DESIGN.md-Ramp („plus one 48px icon glyph"); `--ease-spring`-Token und `::view-transition-group`-Nutzung sind dokumentiert; der Spinner-„Akzent"-Border ist Lade-Technik. Grenzfall: Spring am `IndexingOrbit` — DESIGN.md dokumentiert den Spring nur für View Transitions; entweder dokumentieren oder dort entfernen.

## Overall Impression

Technisch starke Basis (Timeline als sauberes Keyboard-Interface, Mosaik-Adaption, axe 0 violations) — aber die Seite erzählt eine falsche Hierarchie: Der Filter ist der prominenteste Block, und die Informationsschicht der Fotos ist defekt (Overlay hinter dem Bild, mobil gelöscht). Größte Chance: Informationsschicht reparieren und die Timeline demütigen — dann trägt „The Contact Sheet" wieder.

## Was funktioniert

1. **Kontaktbogen-Mosaik + Container-Adaption** (`PhotoGrid.svelte:849/860`): 3×118-px-Kacheln bei 2-px-Fugen auf 390 px; Radien/Rahmen konsequent auf 0 — gelebte Design-System-Treue, gemessen.
2. **Timeline als Signatur** (`TimelineSelector.svelte`): roving `tabbableStart`, `role="slider"`-Handles mit `aria-valuetext`, `role="status" aria-live="polite"`, `aria-disabled` statt `disabled` für leere Perioden, reduced-motion-Block.
3. **A11y-Basis belegt**: `tab.a11y()` (axe-core 4.13) auf der Default-Ansicht: 0 violations, 39 passes; Toast-Trennung `alert`/`status`; Favoriten-Brass exakt nach Grease-Pencil-Regel.

## Priority Issues

### [P0] Karten-Hover zeigt keine Information — Overlay liegt hinter dem Bild
- **Was:** `.photo-card-overlay` hat `z-index: auto`, `.photo-card-image` `z-index: 2` → das Bild malt über den Titel; gemessen: Overlay `opacity: 1`, Titelbox 307×20 px vorhanden, **kein Zeichen sichtbar**. Zusätzlich liegen `.photo-card-actions` (56 px, Band 8–64 px von unten) exakt über `.photo-card-title` (43–63 px) und `.photo-card-meta` (20–37 px).
- **Warum:** DESIGN.md verspricht „fades in title and meta" — real nur ein Fade. Dateiname/Datum/Kamera sind nie sichtbar; mobil (`display:none`) existieren sie gar nicht.
- **Fix:** z-index-Staffel (Bild 1 / Overlay 2 / Actions 3); Titel+Meta aus dem Button-Band heben (`padding-bottom: calc(56px + var(--space-4))`) oder Actions unter die Karte; mobil Titelzeile ausgeben statt ausblenden.
- **Kommando:** `polish`

### [P0] Such-Hinweis und Such-Vorschläge liegen exakt übereinander
- **Was:** `.search-hint` und `.search-suggestions` starten beide bei `top:100%; margin-top:8px` (gemessen y=63) mit gleichem `--glass-bg` (70 %) → „Try: type:video · location:city" scheint durch „Recent search"/„Videos only". B-Beleg: der Placeholder des Felds hat live nur **4,3:1** Kontrast.
- **Warum:** Die häufigste Aktion erzeugt einen unlesbaren Textbrei; die einzige Hilfe im Produkt wird unlesbar, wenn sie gebraucht wird; `location:city` lehrt einen Suchstring, der nie trifft (DE sogar `location:stadt`).
- **Fix:** eine Fläche pro Zustand (Suggestions offen → Hint aus), Vorschläge opak (`--surface-elevated`), Hint auf `--text-secondary` + `aria-describedby`; Platzhalter ≥4,5:1 per Token; `location:`-Beispiel durch echtes Beispiel ersetzen.
- **Kommando:** `layout`

### [P1] Ladezustand existiert visuell und für Screenreader nicht
- **Was:** `.skeleton-item` = `--divider-color`-Gradient → Restfarbe `#eaeaea` auf `#f8f6f3` = **1,117:1**; `#photo-grid` hat null ARIA (`role`, `aria-busy`, Trefferzahl); in einem Live-Lauf blieb die Ansicht dauerhaft bei unsichtbaren Skeletten stehen — von „kaputt" nicht unterscheidbar.
- **Warum:** Erster Eindruck = leere Seite; Screenreader hören beim Laden/Filtern/Suchen nichts; produktweit keine Ergebnis-Zahl.
- **Fix:** Skeleton-Kontrast ≥1,4:1 (oder Shimmer auf `background-secondary`), Text nach 800 ms („Fotos werden geladen…"), `role="region"` + `aria-busy={loading}` am Grid, Trefferzahl-Zeile (`ui.photos_count`).
- **Kommando:** `harden`

### [P1] Timeline-Karte dominiert, zählt falsch, verliert die Zahl und ist im Dark-Mode unlesbar
- **Was:** 210 px hoch (43 % des Viewports), erste Karte bei y=384; Balken 4/9/44 px in 22 % Alpha; Hover auf leere Periode malt `height:0` = nichts, Klick bleibt stumm; `statusText` liefert bei aktiver Selection nur noch das Label (Zahl verschwindet, `TimelineSelector.svelte:166–175`); `.timeline-label.filtered` / `[aria-pressed=true]` im Dark-Theme = **2,18:1** (hell 6,21:1).
- **Warum:** Das größte, gerahmte Element ist ein Filter; die einzige Zahl widerspricht der Suche („17 photos" bei 10 Treffern) und verschwindet beim Filtern; im Dunkelmodus ist der gesetzte Filter nicht lesbar.
- **Fix:** Timeline einklappen/erst ab Spannweite > N Jahre zeigen (URL-State); Balken ≥45 % Alpha + Mindesthöhe; Trefferzahl der gefilterten Liste persistent; Hover-Ansage und Dauerzustand trennen; `--primary-dark` im Dark-Theme überschreiben (≥70 % L).
- **Kommando:** `distill`

### [P1] Mobil: Metadaten und Karten-Aktionen gelöscht, Batch-Aktionen unsichtbar, Ziele zu klein
- **Was:** `@container (<=768px)` setzt `.photo-card-overlay, .photo-card-actions { display:none }`; `#selection-bar` mobil 7 Buttons auf 358 px mit unsichtbarem H-Scroll („Export", „Add to album" de facto unauffindbar); Tap-Ziele 36–38 px (Sort, Select, Jahres-/Monats-Select, Timeline-Reset); Drawer 280 px Glas über buntem Mosaik.
- **Warum:** Die laut PRODUCT.md gleichrangige Ansichtsfläche verliert 2/3 der Kartenfunktionen und alle Metadaten; DESIGN.md fordert „Touch: overlay and actions are permanently visible" und ≥44 px.
- **Fix:** Titel+Meta mobil als Zeile unter der Kachel statt `display:none`; Actions kompakt darunter; Batch-Leiste als Zwei-Reihen-Sheet mit Labels (oder Fade-Edge/Scroll-Snap); `.btn.danger` (Signal Red) für Löschen + Dialog statt `confirm()`; Min. 44 px für die vier Controls; Drawer mobil `--surface-color`.
- **Kommando:** `adapt`

## Persona-Red-Flags

**Jordan (First-Timer)**
- Suchfeld fokussieren ⇒ Hint und Suggestions übereinander (unlesbarer Brei) — erste Berührung mit der Suche.
- Dass die Suche semantisch ist (Produktversprechen), steht nirgends; Platzhalter sagt nur „Search photos…".
- Hover auf Kachel = zwei unbeschriftete Kreise, kein Dateiname — „was ist das?" bleibt offen.
- Drei Namen für die Bibliothek: „All media" (Nav), „17 photos" (Zähler), „No Photos Found" (Leerzustand).

**Sam (Accessibility)**
- `#photo-grid` ohne `role`/`aria-busy`/Live-Zahl — Lade-/Filterergebnisse stumm (axe findet das nicht).
- `#search-input` kein Combobox: kein `role="combobox"`, kein `aria-expanded`/`aria-activedescendant`; ArrowDown in Vorschläge unmöglich.
- `:focus-visible` fehlt komplett für `.nav-item`, `.search-btn`, `.save-search-btn`, `.load-more-btn`, `.menu-btn`, `.theme-toggle`; Suchfeld/Sort setzen `outline:none` und ändern nur die 1-px-Rahmenfarbe.
- Dark-Theme: Filter-Chip 2,18:1, `--text-muted` 3,02:1 auf 13 px.
- ~21 Tab-Stops vor der ersten Fotokarte.

**Casey (Mobile)**
- Keine Metadaten, kein Favorit/Download pro Kachel (Container-Query).
- Batch-Leiste: 4 von 7 Aktionen nur per verstecktem H-Scroll.
- Tap-Ziele 36–38 px unter dem 44-px-Systemanspruch.
- Drawer bleibt Glas über farbigem Mosaik; Nav-Labels auf weichgezeichneten Fotos.

## Minor Observations

- **Karten stretchen bei wenigen Treffern:** `min-height: 400px` + `align-items: stretch` → Einzelkarte 349×400 px bei quadratischem Bild → 53 px tote Fläche; `align-items: start` hilft.
- **Viewport- statt Container-Breakpoints** im Grid (`@media >=1400 / <=1200 / <=1024`), nur Mobil ist `@container` — DESIGN.md behauptet „adapts to its container".
- **Plural fehlt überall:** „1 photos" (aria-labels, Status, DE „1 Fotos") — kein ICU-`{count}`-String existiert.
- **Kein `::placeholder`-Styling** im Projekt; die Feld-Farbe kommt vom Browser (Beleg: 4,3:1).
- **Tote Tokens:** `--color-bg` (3×), `--shadow-lg` (2×) — Dialoge weiterhin ohne Schatten.
- **Hartkodierte Farben:** `#ef4444`/`rgb(239 68 68 / …)` (app.css:1363–1366, ViewerMetadataEdit.svelte:491–494).
- **`--text-muted` ist im Dark-Theme dunkler** (50 % L) als im Light-Theme (60 %) — schwächste Farbe dort, wo sie am schwächsten wirkt.
- **Bild-Platzhalter-Icon unsichtbar:** `stroke='currentColor'` in Data-URI kann nicht auflösen → schwarzes Kamera-Icon, im Dark-Theme praktisch unsichtbar; `.error-placeholder`-Rot greift nie.
- **Reduced-Transparency-Lücken:** Hint, Suggestions, `#selection-bar` (Text auf Glas) haben keinen Solid-Fallback.
- **Mobile Suche überdeckt Inhalt** (h2/Toolbar); Video-Badge 48 px = 41 % mobiler Kachelbreite; 1-Zeichen-Query gibt keine Rückmeldung (Debounce ≥2).

## Questions to Consider

1. **Wenn der einzige gerahmte Gegenstand der Filter ist — wer ist hier der Inhalt?** Sollte die Timeline erst ab einer Spannweite sichtbar sein, bei der „Dichte über Zeit" wirklich etwas aussagt?
2. **Was gewinnt der Nutzer durch einen Hover, der nur zwei unbeschriftete Kreise zeigt?** Wäre die Kachel reine Lese-Fläche und alle Actionen erst im Viewer?
3. **Welchen Wert hat eine Suche, die nie „nein" sagt?** Zehn selbstbewusste Treffer für Unsinn — Feature oder Vertrauensschuld?
