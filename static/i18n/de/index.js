window.DE_TRANSLATIONS = {
  ui: {
    appTitle: 'TurboPix',
    search: 'Suchen',
    search_photos_placeholder: 'Fotos suchen...',
    search_ai_placeholder: 'KI-gestützte Fotosuche...',
    all_photos: 'Alle Medien',
    favorites: 'Favoriten',
    videos: 'Videos',
    collages: 'Collagen',
    housekeeping: 'Aufräumen', // Keep german translation but key changed
    favorite_photos: 'Lieblingsfotos',
    load_more: 'Mehr laden',
    loading: 'Wird geladen...',
    all_photos_loaded: 'Alle Fotos geladen',
    no_photos_found: 'Keine Fotos gefunden',
    try_again: 'Erneut versuchen',
    refresh: 'Aktualisieren',
    added: 'Hinzugefügt',
    removed: 'Entfernt',
    download: 'Herunterladen',
    menu: 'Menü',
    connection: 'Verbindung',

    // Sorting
    newest_first: 'Neueste zuerst',
    oldest_first: 'Älteste zuerst',
    name_a_z: 'Name A-Z',
    name_z_a: 'Name Z-A',
    largest_first: 'Größte zuerst',
    smallest_first: 'Kleinste zuerst',

    // Timeline
    all_dates: 'Alle Daten',
    all_years: 'Alle Jahre',
    all_months: 'Alle Monate',
    clear_timeline_filter: 'Zeitfilter löschen',
    timeline_first: 'Erste',
    timeline_prev: 'Vorheriger Monat',
    timeline_next: 'Nächster Monat',
    timeline_last: 'Letzte (Alle Daten)',
    photos_count: '{{count}} Fotos',

    // Photo viewer
    photo_title: 'Foto Titel',
    photo: 'Foto',
    date: 'Datum:',
    size: 'Größe:',
    camera: 'Kamera:',
    location: 'Ort:',
    favorite: 'Favorit',
    share: 'Teilen',
    close: 'Schließen',
    previous: 'Vorheriges',
    next: 'Nächstes',
    zoom_out: 'Verkleinern',
    zoom_in: 'Vergrößern',
    fit_to_screen: 'An Bildschirm anpassen',
    fullscreen: 'Vollbild',
    toggle_info: 'Info umschalten',
    view_details: 'Details anzeigen',

    // Buttons and actions
    add_to_favorites: 'Zu Favoriten hinzufügen',
    remove_from_favorites: 'Von Favoriten entfernen',
    toggle_theme: 'Design wechseln',
    rotate_left: 'Links drehen 90°',
    rotate_right: 'Rechts drehen 90°',
    flip_horizontal: 'Horizontal spiegeln',
    flip_vertical: 'Vertikal spiegeln',
    delete_photo: 'Foto löschen',
    accept_collage: 'Akzeptieren',
    reject_collage: 'Ablehnen',
    collages_load_failed: 'Collagen konnten nicht geladen werden',
    collage_for: 'Collage für {{date}}',

    // Values
    unknown: 'Unbekannt',
    no_location_data: 'Keine Ortsdaten',
    yes: 'Ja',
    no: 'Nein',

    // Months
    months: {
      january: 'Januar',
      february: 'Februar',
      march: 'März',
      april: 'April',
      may: 'Mai',
      june: 'Juni',
      july: 'Juli',
      august: 'August',
      september: 'September',
      october: 'Oktober',
      november: 'November',
      december: 'Dezember',
    },
    weekdays: {
      sunday: 'Sonntag',
      monday: 'Montag',
      tuesday: 'Dienstag',
      wednesday: 'Mittwoch',
      thursday: 'Donnerstag',
      friday: 'Freitag',
      saturday: 'Samstag',
    },

    // Metadata panel
    metadata: {
      file_information: 'Dateiinformationen',
      file_path: 'Dateipfad:',
      file_size: 'Dateigröße:',
      dimensions: 'Abmessungen:',
      type: 'Typ:',
      date_taken: 'Aufnahmedatum:',
      date_modified: 'Änderungsdatum:',

      camera_section: 'Kamera',
      make: 'Hersteller:',
      model: 'Modell:',
      lens_make: 'Objektivhersteller:',
      lens_model: 'Objektivmodell:',

      camera_settings: 'Kameraeinstellungen',
      iso: 'ISO:',
      aperture: 'Blende:',
      shutter_speed: 'Verschlusszeit:',
      focal_length: 'Brennweite:',
      exposure_mode: 'Belichtungsmodus:',
      metering_mode: 'Messmodus:',
      white_balance: 'Weißabgleich:',
      flash: 'Blitz:',
      orientation: 'Ausrichtung:',
      color_space: 'Farbraum:',

      location_section: 'Standort',
      gps: 'GPS:',
      location_name: 'Ortsname:',

      video_section: 'Videoinformationen',
      duration: 'Dauer:',
      video_codec: 'Video-Codec:',
      audio_codec: 'Audio-Codec:',
      frame_rate: 'Bildrate:',
      bitrate: 'Bitrate:',

      // Metadata editing
      edit_button: 'Metadaten bearbeiten',
      edit_modal_title: 'Foto-Metadaten bearbeiten',
      edit_date_label: 'Aufnahmedatum',
      edit_latitude_label: 'Breitengrad',
      edit_longitude_label: 'Längengrad',
      edit_cancel: 'Abbrechen',
      edit_save: 'Speichern',
      edit_success: 'Metadaten erfolgreich aktualisiert',
      edit_error: 'Metadaten konnten nicht aktualisiert werden',
      edit_validation_date: 'Ungültiges Datumsformat',
      edit_validation_gps:
        'GPS-Koordinaten müssen zwischen -90/90 (Breitengrad) und -180/180 (Längengrad) liegen',
      edit_validation_gps_pair: 'Breitengrad und Längengrad müssen zusammen angegeben werden',
      edit_unsupported_format:
        'Die Bearbeitung von {{format}}-Dateien wird nicht unterstützt. Nur JPEG- und PNG-Formate werden für die Metadatenbearbeitung unterstützt.',
    },

    // Search
    search_results: 'Suche: "{{query}}"',
    recent_search: 'Letzte Suche',
    filter_by_camera: 'Nach Kamera filtern',
    filter_by_type: 'Nach Typ filtern',
    has_gps_data: 'Hat GPS-Daten',
    videos_only: 'Nur Videos',
    canon_photos: 'Canon-Fotos',
    nikon_photos: 'Nikon-Fotos',
    sony_photos: 'Sony-Fotos',
    photos_with_location: 'Fotos mit Standort',
    photos_from_year: 'Fotos aus {{year}}',
    raw_files_only: 'Nur RAW-Dateien',
    photos_with_gps: 'Fotos mit GPS',

    // Indexing
    indexing_photos: 'Fotos werden indexiert...',
    indexing_housekeeping: 'Identifiziere Aufräumkandidaten...',

    // Indexing phases (labels)
    indexing_phase_discovering: 'Entdecken',
    indexing_phase_metadata: 'Metadaten',
    indexing_phase_semantic: 'Semantisch',
    indexing_phase_collages: 'Collagen',
    indexing_phase_housekeeping: 'Aufräumen',

    // Indexing status messages
    indexing_status_discovering: 'Dateien werden entdeckt...',
    indexing_status_metadata: 'Metadaten werden indexiert...',
    indexing_status_semantic: 'Semantische Vektoren werden berechnet...',
    indexing_status_collages: 'Collagen werden erstellt...',
    indexing_status_housekeeping: 'Aufräumkandidaten werden identifiziert...',

    // Indexing counter template
    indexing_counter: '{{processed}} / {{total}}',

    // Collages
    no_pending_collages: 'Keine ausstehenden Collagen',
    collage_date: 'Datum:',
    collage_photos: '{{count}} Fotos',
  },

  errors: {
    photoNotFound: 'Foto nicht gefunden',
    databaseError: 'Datenbankfehler aufgetreten',
    searchError: 'Suche fehlgeschlagen',
    failedToLoadPhoto: 'Foto konnte nicht geladen werden',
    failedToLoadImage: 'Bild konnte nicht geladen werden',
    failedToReadPhotoFile: 'Fotodatei konnte nicht gelesen werden',
    invalidThumbnailSize: 'Ungültige Miniaturbildgröße',
    unexpectedError: 'Ein unerwarteter Fehler ist aufgetreten',
    connectionLost: 'Serververbindung verloren',
  },

  notifications: {
    photoAddedToFavorites: 'Foto zu Favoriten hinzugefügt',
    photoRemovedFromFavorites: 'Foto von Favoriten entfernt',
    downloadStarted: 'Foto-Download gestartet',
    sharedSuccessfully: 'Foto erfolgreich geteilt',
    sharingCancelled: 'Teilen abgebrochen oder nicht unterstützt',
    collageAccepted: 'Collage akzeptiert. Wird nach dem nächsten Scan in Alle Fotos angezeigt.',
    collageRejected: 'Collage abgelehnt und gelöscht',
    collageAcceptFailed: 'Collage konnte nicht akzeptiert werden',
    collageRejectFailed: 'Collage konnte nicht abgelehnt werden',
    collagesGenerated: '{{count}} Collage(n) erfolgreich generiert',
    collageGenerateFailed: 'Collagen konnten nicht generiert werden',

    // Toast titles
    added: 'Hinzugefügt',
    removed: 'Entfernt',
    download: 'Herunterladen',
    shared: 'Geteilt',
    share: 'Teilen',
    error: 'Fehler',
    connection: 'Verbindung',
  },

  messages: {
    no_photos_indexed: 'Es wurden noch keine Fotos indiziert',
    no_photos_match_search: 'Keine Fotos entsprechen Ihrer Suche nach "{{query}}"',
    photo_added_to_favorites: 'Foto zu Favoriten hinzugefügt',
    photo_removed_from_favorites: 'Foto von Favoriten entfernt',
    error_updating_favorite: 'Fehler beim Aktualisieren des Favoritenstatus',
    photo_download_started: 'Download gestartet',
    confirm_reject_collage: 'Möchten Sie diese Collage wirklich ablehnen?',
  },

  video: {
    transcoding: {
      started: 'Video wird für die Wiedergabe konvertiert...',
      completed: 'Videokonvertierung abgeschlossen',
      failed: 'Videokonvertierung fehlgeschlagen',
      timeout: 'Videokonvertierung: Zeitlimit überschritten',
    },
  },
};
