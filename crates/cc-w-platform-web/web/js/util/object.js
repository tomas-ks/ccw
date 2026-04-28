export function tryGetFirst(value, candidates) {
  for (const key of candidates) {
    if (value && value[key] !== undefined && value[key] !== null) {
      return value[key];
    }
  }
  return null;
}
