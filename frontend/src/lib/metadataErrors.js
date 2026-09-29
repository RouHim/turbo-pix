// Maps the metadata PATCH endpoint's refusal `error_code` values to the i18n
// key holding the user-facing message. Keep in sync with the backend's code
// list; tests/metadata-errors.test.js guards both the codes and the keys.
export const METADATA_ERROR_KEYS = {
  invalid_date: 'ui.metadata.edit_error_invalid_date',
  invalid_coordinates: 'ui.metadata.edit_error_invalid_coordinates',
  unsupported_container: 'ui.metadata.edit_error_unsupported_container',
  no_location_carrier: 'ui.metadata.edit_error_no_location_carrier',
  no_writable_slot: 'ui.metadata.edit_error_no_writable_slot',
  unrepresentable_value: 'ui.metadata.edit_error_unrepresentable_value',
  file_read_only: 'ui.metadata.edit_error_file_read_only',
  file_missing: 'ui.metadata.edit_error_file_missing',
};
