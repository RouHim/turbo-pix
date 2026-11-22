export default {
  compilerOptions: { runes: true },
  // The repo's zero-warnings policy applies to the frontend compiler too:
  // Vite otherwise prints Svelte warnings and exits 0, so CI never sees them.
  onwarn(warning) {
    const location = warning.filename
      ? `${warning.filename}:${warning.start?.line ?? '?'}:${warning.start?.column ?? '?'}`
      : 'unknown location';
    throw new Error(`Svelte ${warning.code}: ${warning.message} (${location})`);
  },
};
